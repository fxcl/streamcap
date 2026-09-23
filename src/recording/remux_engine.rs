//! FFmpeg remux engine (read packet loop, interrupt callback, reconnect).
#![allow(dead_code)]

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use rsmpeg::avcodec::AVCodecParameters;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVRational;
use rsmpeg::shared::AVERROR_EXIT;
use tracing::{debug, error, info, warn};

use crate::format::{FormatBuilder, FormatConfig};
use crate::recording::input_options::InputOptions;
use crate::recording::error::RecordingError;
use crate::recording::RecorderStats;

/// 基于 rsmpeg 的 remux 录制引擎 - 包含重连和即时中断能力
pub struct RemuxEngine {
    pub(crate) stop_signal: Arc<AtomicBool>,
}

impl Default for RemuxEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RemuxEngine {
    pub fn new() -> Self {
        Self {
            stop_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn request_stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }

    /// 执行 remux 录制 (主入口)
    pub fn remux(
        &self,
        input_url: &str,
        output_path: &str,
        format_config: &FormatConfig,
        input_options: &InputOptions,
        max_retries: u32,
    ) -> Result<RecorderStats, RecordingError> {
        let start_time = Instant::now();
        let mut stats = RecorderStats::default();
        let mut retry_count = 0u32;
        let mut total_packets: u64 = 0;

        info!("Starting remux: {} -> {}", input_url, output_path);

        loop {
            if self.stop_signal.load(Ordering::SeqCst) {
                info!("Stop requested before starting remux");
                break;
            }

            match self.remux_single_session(
                input_url,
                output_path,
                format_config,
                input_options,
                total_packets,
            ) {
                Ok(packets) => {
                    total_packets = packets;
                    info!("Stream ended cleanly after {} packets", total_packets);
                    break;
                }
                Err(RecordingError::StreamInterrupted) => {
                    info!("Recording interrupted by user after {} packets", total_packets);
                    break;
                }
                Err(e) => {
                    error!("Session error: {}", e);
                    if retry_count >= max_retries {
                        error!("Max retries ({}) exceeded, giving up", max_retries);
                        return Err(e);
                    }
                    retry_count += 1;
                    let delay_secs = std::cmp::min(5u64 * retry_count as u64, 30);
                    warn!(
                        "Retrying in {}s (attempt {}/{})",
                        delay_secs, retry_count, max_retries
                    );

                    // 等待重连, 期间监听停止信号
                    let deadline = Instant::now() + std::time::Duration::from_secs(delay_secs);
                    while Instant::now() < deadline {
                        if self.stop_signal.load(Ordering::SeqCst) {
                            info!("Stop requested during retry wait");
                            return Err(RecordingError::StoppedByUser);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        }

        stats.total_packets = total_packets;
        stats.duration_secs = start_time.elapsed().as_secs();
        stats.bytes_written = std::fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
        stats.cumulative_bytes = stats.bytes_written;
        if stats.duration_secs > 0 {
            stats.bytes_per_sec = stats.bytes_written / stats.duration_secs;
        }
        stats.speed_string = format_speed(stats.bytes_per_sec);

        info!(
            "Remux complete: {} packets in {}s, {} bytes ({} retries)",
            stats.total_packets, stats.duration_secs, stats.bytes_written, retry_count
        );

        Ok(stats)
    }

    /// 单次 remux 会话: 打开输入 → remux → 写trailer
    fn remux_single_session(
        &self,
        input_url: &str,
        output_path: &str,
        format_config: &FormatConfig,
        input_options: &InputOptions,
        previous_packets: u64,
    ) -> Result<u64, RecordingError> {
        let mut input_ctx = self.open_input_with_interrupt(input_url, input_options)?;

        let input_streams_data: Vec<(AVCodecParameters, AVRational)> = {
            let streams = input_ctx.streams();
            if streams.is_empty() {
                return Err(RecordingError::NoStreamFound);
            }
            info!("Input has {} streams", streams.len());
            streams
                .iter()
                .map(|s| (s.codecpar().clone(), s.time_base))
                .collect()
        };

        for (i, (params, _tb)) in input_streams_data.iter().enumerate() {
            debug!(
                "  Stream {}: codec_type={:?}, codec_id={}",
                i, params.codec_type, params.codec_id
            );
        }

        let output_filename = CString::new(output_path)
            .map_err(|e| RecordingError::FormatError(format!("Invalid output path: {}", e)))?;

        let format_builder = FormatBuilder::new(format_config.clone());
        let mut output_ctx =
            format_builder.build_output_context(&output_filename, &input_streams_data)?;

        let mut write_header_dict = format_config.write_header_dict();
        output_ctx.write_header(&mut write_header_dict)?;

        let mut packet_count: u64 = 0;

        // 记录输入流的时间基 (用于 PTS rescale)
        let input_timebases: Vec<rsmpeg::ffi::AVRational> = input_ctx
            .streams()
            .iter()
            .map(|s| s.time_base)
            .collect();

        // 前 PHASE_OFFSET_PROBE 帧用作 PTS 探测 — 找出各流的独立 min PTS
        // 注意: 不同流的 time_base 不同 (如音频 1/48000, 视频 1/1000),
        // 不能用全局 min PTS, 必须按 stream_index 独立归一
        const PHASE_OFFSET_PROBE: i64 = 200;
        let mut probe_buffer: Vec<rsmpeg::avcodec::AVPacket> = Vec::with_capacity(200);
        use std::collections::HashMap;
        let mut stream_min_pts: HashMap<i32, i64> = HashMap::new();

        // 归一化辅助闭包
        let normalize_write = |packet: &mut rsmpeg::avcodec::AVPacket,
                               ctx: &mut rsmpeg::avformat::AVFormatContextOutput,
                               stream_mins: &HashMap<i32, i64>,
                               in_tbs: &[rsmpeg::ffi::AVRational]|
         -> Result<(), RecordingError> {
            let stream_index = packet.stream_index as usize;
            if stream_index < ctx.streams().len() {
                let out_tb = ctx.streams()[stream_index].time_base;
                let in_tb = in_tbs.get(stream_index).copied().unwrap_or(
                    rsmpeg::ffi::AVRational { num: 1, den: 1000 },
                );
                if let Some(&min_pts) = stream_mins.get(&packet.stream_index) {
                    if packet.pts != rsmpeg::ffi::AV_NOPTS_VALUE {
                        let pts_off = packet.pts - min_pts;
                        packet.set_pts(pts_off);
                    }
                    if packet.dts != rsmpeg::ffi::AV_NOPTS_VALUE {
                        let dts_off = packet.dts - min_pts;
                        packet.set_dts(dts_off);
                    }
                }
                packet.rescale_ts(in_tb, out_tb);
                ctx.interleaved_write_frame(packet)?;
            }
            Ok(())
        };

        loop {
            // Probe 阶段: 缓存 packet 并记录各流最小 PTS (独立于 time_base)
            if packet_count < PHASE_OFFSET_PROBE as u64 {
                match input_ctx.read_packet() {
                    Ok(Some(packet)) => {
                        if packet.pts != rsmpeg::ffi::AV_NOPTS_VALUE {
                            let sid = packet.stream_index;
                            let entry = stream_min_pts.entry(sid).or_insert(packet.pts);
                            if packet.pts < *entry {
                                *entry = packet.pts;
                            }
                        }
                        packet_count += 1;
                        probe_buffer.push(packet);
                        continue;
                    }
                    other => {
                        // 流提前中断 (probe 阶段还没收够), 写出已缓存部分
                        for mut pkt in probe_buffer.drain(..) {
                            normalize_write(&mut pkt, &mut output_ctx, &stream_min_pts, &input_timebases)?;
                        }
                        match other {
                            Ok(None) => {
                                info!("End of stream reached after {} packets (probe phase)", previous_packets + packet_count);
                                break;
                            }
                            Err(e) => {
                                let is_exit = is_averror_exit(&e);
                                if is_exit {
                                    warn!("Recording interrupted by callback (probe phase)");
                                    output_ctx.write_trailer()?;
                                    return Err(RecordingError::StreamInterrupted);
                                }
                                error!("Error reading packet: {} (will reconnect)", e);
                                output_ctx.write_trailer()?;
                                return Err(RecordingError::from(e));
                            }
                            _ => {}
                        }
                        break;
                    }
                }
            }

            // Probe 结束, 写出缓存的 packet
            if !probe_buffer.is_empty() {
                for mut pkt in probe_buffer.drain(..) {
                    normalize_write(&mut pkt, &mut output_ctx, &stream_min_pts, &input_timebases)?;
                }
            }

            match input_ctx.read_packet() {
                Ok(Some(mut packet)) => {
                    packet_count += 1;
                    if packet_count % 500 == 0 {
                        info!(
                            "Recorded {} packets, elapsed {}s",
                            previous_packets + packet_count,
                            (Instant::now()).elapsed().as_secs()
                        );
                    }

                    normalize_write(&mut packet, &mut output_ctx, &stream_min_pts, &input_timebases)?;
                }
                Ok(None) => {
                    info!(
                        "End of stream reached after {} packets",
                        previous_packets + packet_count
                    );
                    break;
                }
                Err(e) => {
                    let is_exit = is_averror_exit(&e);
                    if is_exit {
                        warn!("Recording interrupted by callback");
                        output_ctx.write_trailer()?;
                        return Err(RecordingError::StreamInterrupted);
                    }
                    error!("Error reading packet: {} (will reconnect)", e);
                    output_ctx.write_trailer()?;
                    return Err(RecordingError::from(e));
                }
            }
        }

        output_ctx.write_trailer()?;

        Ok(previous_packets + packet_count)
    }

    /// 打开输入流 + 设置 interrupt_callback 实现即时停止
    fn open_input_with_interrupt(
        &self,
        url: &str,
        input_options: &InputOptions,
    ) -> Result<AVFormatContextInput, RecordingError> {
        let url_cstring = CString::new(url)
            .map_err(|e| RecordingError::InvalidUrl(format!("Invalid URL: {}", e)))?;

        let dict = input_options.to_avdictionary();
        let mut opts = Some(dict);

        let mut input_ctx = AVFormatContextInput::builder()
            .url(&url_cstring)
            .options(&mut opts)
            .open()?;

        // 设置中断回调: 当 stop_signal 为 true 时, 立即退出阻塞 IO
        let stop = Arc::clone(&self.stop_signal);
        input_ctx.set_interrupt_callback(move || stop.load(Ordering::Relaxed));

        info!("Successfully opened input with interrupt callback: {}", url);
        Ok(input_ctx)
    }
}

/// 检查 RsmpegError 是否为 AVERROR_EXIT (interrupt_callback 中断)
fn is_averror_exit(e: &rsmpeg::error::RsmpegError) -> bool {
    use rsmpeg::error::RsmpegError::*;
    match e {
        AVError(code) => *code == AVERROR_EXIT,
        _ => false,
    }
}

/// 格式化速度为人类可读字符串
fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1024 * 1024 {
        format!("{:.1} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{} B/s", bytes_per_sec)
    }
}

/// 文件名模板系统 - 对应 Python 中的 save_path 格式
#[allow(dead_code)]
pub struct FilenameTemplate {
    template: String,
}

impl FilenameTemplate {
    pub fn new(template: impl Into<String>) -> Self {
        Self {
            template: template.into(),
        }
    }

    pub fn default_template() -> Self {
        Self::new("{anchor_name}/{platform}_{time}")
    }

    /// 渲染为最终路径
    pub fn render(
        &self,
        anchor_name: &str,
        title: &str,
        platform: &str,
        output_dir: &str,
        extension: &str,
    ) -> String {
        let now = time::OffsetDateTime::now_local().unwrap_or_else(|e| {
            warn!("Failed to get local time, using UTC: {}", e);
            time::OffsetDateTime::now_utc()
        });

        let time_str = format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
            now.second()
        );

        let date_str = format!(
            "{:04}-{:02}-{:02}",
            now.year(),
            now.month() as u8,
            now.day()
        );

        let cleaned_anchor = crate::utils::clean_filename(anchor_name);
        let cleaned_title = crate::utils::clean_filename(title);
        let cleaned_platform = crate::utils::clean_filename(platform);

        let result = self
            .template
            .replace("{anchor_name}", &cleaned_anchor)
            .replace("{title}", &cleaned_title)
            .replace("{platform}", &cleaned_platform)
            .replace("{time}", &time_str)
            .replace("{date}", &date_str);

        let output_path = std::path::Path::new(output_dir).join(&result);
        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if extension.is_empty() {
            output_path.to_string_lossy().to_string()
        } else {
            format!("{}.{}", output_path.display(), extension)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording::input_options::InputOptions;

    #[test]
    fn test_engine_creation() {
        let engine = RemuxEngine::new();
        assert!(!engine.stop_signal.load(Ordering::SeqCst));
        engine.request_stop();
        assert!(engine.stop_signal.load(Ordering::SeqCst));
    }

    #[test]
    fn test_filename_template() {
        let tpl = FilenameTemplate::new("{anchor_name}/{time}");
        let path = tpl.render("Test主播", "Title", "douyin", "/tmp/recordings", "mp4");
        assert!(path.contains("Test"));
        assert!(path.ends_with(".mp4"));
        assert!(path.contains("/tmp/recordings/"));
    }

    #[test]
    fn test_input_options_to_dict() {
        let opts = InputOptions::default_cn();
        let dict = opts.to_avdictionary();
        let _ = format!("{:?}", dict);
    }
}
