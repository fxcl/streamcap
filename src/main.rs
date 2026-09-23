mod config;
mod format;
mod i18n;
mod models;
mod notifications;
mod platform;
mod recording;
mod utils;
mod web_api;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use rsmpeg::avcodec::AVCodecParameters;
use rsmpeg::avutil::AVRational;
use tracing::{error, info};

use crate::config::ConfigManager;
use crate::format::OutputFormat;
use crate::i18n::t;
use crate::notifications::{MessagePusher, NotificationEvent, PushConfig};
use crate::platform::PlatformRegistry;
use crate::recording::RecordingController;
use crate::recording::stream_recorder::{RecorderConfig, StreamRecorder};

#[derive(Parser, Debug)]
#[command(name = "streamcap", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, default_value = ".")]
    config_dir: PathBuf,

    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Record {
        #[arg(short, long)]
        url: String,

        #[arg(short, long)]
        output: Option<String>,

        /// 输出容器格式 (默认 ts: 直播流天然格式, 中断即可播放; mp4: 需等待 trailer, 否则可能损坏)
        #[arg(short, long, default_value = "ts")]
        format: String,

        #[arg(long)]
        segment: bool,

        #[arg(long, default_value = "1800")]
        segment_time: u32,

        #[arg(long)]
        overseas: bool,

        #[arg(long)]
        direct_download: bool,

        #[arg(long)]
        headers: Option<String>,

        #[arg(long)]
        proxy: Option<String>,
    },

    /// 启动 HTTP API 服务 (对齐 Python video_stream_service.py)
    Serve {
        /// 监听地址 (默认 0.0.0.0:6007)
        #[arg(short, long, default_value = "0.0.0.0:6007")]
        addr: String,

        /// 视频文件根目录
        #[arg(short, long, default_value = "./recordings")]
        video_dir: String,

        /// 录制检查间隔 (秒)
        #[arg(long, default_value = "180")]
        check_interval: u64,
    },

    List,

    Init,

    Demo {
        url: String,

        output: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .with_target(true)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .init();

    info!("StreamCap (Rust) starting...");

    match cli.command {
        Commands::Record {
            url,
            output,
            format,
            segment,
            segment_time,
            overseas,
            direct_download,
            headers,
            proxy,
        } => {
            cmd_record(
                &cli.config_dir,
                &url,
                output,
                &format,
                segment,
                segment_time,
                overseas,
                direct_download,
                headers,
                proxy,
            ).await?;
        }
        Commands::List => {
            cmd_list(&cli.config_dir).await?;
        }
        Commands::Init => {
            cmd_init(&cli.config_dir).await?;
        }
        Commands::Serve {
            addr,
            video_dir,
            check_interval,
        } => {
            cmd_serve(&addr, &video_dir, check_interval).await?;
        }
        Commands::Demo { url, output } => {
            cmd_demo(&url, &output).await?;
        }
    }

    Ok(())
}

async fn cmd_record(
    config_dir: &PathBuf,
    url: &str,
    output: Option<String>,
    format_str: &str,
    segment: bool,
    segment_time: u32,
    overseas: bool,
    direct_download: bool,
    headers: Option<String>,
    proxy: Option<String>,
) -> anyhow::Result<()> {
    use std::io::Write;

    let format: OutputFormat = format_str.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let config_mgr = ConfigManager::new(config_dir).await?;
    let settings = config_mgr.load_settings().await;

    // ── 平台自动识别 + 获取真实流 URL ───────────────────────
    let mut registry = PlatformRegistry::with_defaults();
    if let Some(ref p) = proxy {
        registry.set_proxy(Some(p.clone()));
    }
    let platform_name = registry
        .resolve(url)
        .map(|h| h.platform_name().to_string())
        .unwrap_or_else(|| "未知".to_string());
    info!("Platform detected: {} for URL: {}", platform_name, url);

    // 获取真实流地址 (调用平台 API)
    let stream_data = registry.fetch_stream_data(url).await;
    let record_url = match &stream_data {
        Some(sd) => {
            let real_url = sd.best_record_url().unwrap_or(url);
            info!("Real stream URL: {}", real_url);
            if !sd.title.is_empty() {
                info!("Live title: {}", sd.title);
            }
            if !sd.anchor_name.is_empty() {
                info!("Anchor: {}", sd.anchor_name);
            }
            real_url.to_string()
        }
        None => {
            info!("Could not fetch stream info, using URL directly");
            url.to_string()
        }
    };

    // ── 通知系统 ────────────────────────────────────────────
    let push_config = PushConfig::default();
    let pusher = MessagePusher::new(push_config);
    pusher.notify(&NotificationEvent::LiveStart {
        anchor_name: platform_name.clone(),
        platform: platform_name.clone(),
        title: String::new(),
        url: url.to_string(),
    }).await;

    let output_path = match output {
        Some(o) => o,
        None => generate_default_output(&settings.video_save_path, url, &format),
    };

    let recorder_config = RecorderConfig {
        record_url,
        output_path: output_path.clone(),
        format,
        segment_record: segment,
        segment_time,
        is_overseas: overseas,
        headers,
        proxy: proxy.clone(),
        use_direct_download: direct_download,
        input_options: None,
        max_retries: if overseas { 5 } else { 3 },
    };

    info!("Recording {} -> {}", url, output_path);

    let recorder = Arc::new(StreamRecorder::new());
    let recorder_clone = Arc::clone(&recorder);
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Received Ctrl+C, stopping recording...");
        recorder_clone.stop().await;
    });

    let pusher_ref = &pusher;
    match recorder.start(recorder_config).await {
        Ok(()) => {
            while recorder.is_recording().await {
                if let Some(secs) = recorder.elapsed_secs().await {
                    print!("\rRecording [{}] {} ", platform_name, crate::utils::format_duration(secs));
                    let _ = std::io::stdout().flush();
                }
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
            let total_secs = recorder.elapsed_secs().await.unwrap_or(0);
            println!("\nRecording finished: {} -> {} ({}s)", url, output_path, total_secs);

            let platform_for_notify = platform_name.clone();
            pusher_ref.notify(&NotificationEvent::LiveEnd {
                anchor_name: platform_for_notify.clone(),
                platform: platform_for_notify,
                duration_secs: total_secs,
                bytes_written: 0,
            }).await;

            // 保存任务到 config
            let save_mgr = ConfigManager::new(config_dir).await?;
            let rec = crate::models::Recording {
                rec_id: format!("{:08x}", std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()),
                url: url.to_string(),
                quality: "OD".to_string(),
                record_format: format_str.to_string(),
                monitor_status: false,
                segment_record: segment,
                segment_time: segment_time.to_string(),
                streamer_name: String::new(),
                scheduled_recording: false,
                scheduled_start_time: String::new(),
                monitor_hours: String::new(),
                recording_dir: String::new(),
                enabled_message_push: false,
                only_notify_no_record: false,
                flv_use_direct_download: false,
                video_bitrate: None,
                platform: Some(platform_name.clone()),
                platform_key: None,
                title: String::new(),
                display_title: String::new(),
                last_duration: format!("{}s", total_secs),
            };
            save_mgr.save_recordings(&[rec]).await?;
        }
        Err(e) => {
            error!("Recording failed: {}", e);
            pusher_ref.notify(&NotificationEvent::RecordingFailed {
                anchor_name: platform_name.clone(),
                platform: platform_name,
                reason: e.to_string(),
            }).await;
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn cmd_list(config_dir: &PathBuf) -> anyhow::Result<()> {
    let config_mgr = ConfigManager::new(config_dir).await?;
    let recordings = config_mgr.load_recordings().await?;

    if recordings.is_empty() {
        println!("No recordings configured.");
    } else {
        println!("Configured recordings ({} total):", recordings.len());
        for rec in &recordings {
            let status_str = if rec.monitor_status {
                crate::models::RecordingStatus::Monitoring.description()
            } else {
                crate::models::RecordingStatus::StoppedMonitoring.description()
            };
            println!("  [{}] {} [{}] - {} ({})",
                &rec.rec_id[..8.min(rec.rec_id.len())],
                crate::utils::clean_filename(&rec.streamer_name),
                status_str,
                rec.url,
                rec.quality,
            );
        }
    }

    Ok(())
}

async fn cmd_init(config_dir: &PathBuf) -> anyhow::Result<()> {
    let config_mgr = ConfigManager::new(config_dir).await?;
    let settings = config_mgr.load_settings().await;
    println!("Config dir: {:?}", config_mgr.config_dir());
    println!("{}: {}", t("video_save_path"), settings.video_save_path);
    println!("{}: {} {}", t("loop_time"), settings.loop_time_seconds, t("seconds"));

    // 保存初始配置
    config_mgr.save_settings(&settings).await?;
    info!("Settings saved to {:?}", config_mgr.config_dir());

    Ok(())
}

async fn cmd_demo(url: &str, output: &str) -> anyhow::Result<()> {
    info!("Demo: remuxing {} -> {}", url, output);

    use std::ffi::CString;
    use rsmpeg::avformat::AVFormatContextInput;

    let url_c = CString::new(url)?;
    let output_c = CString::new(output)?;

    let mut input_ctx = AVFormatContextInput::builder()
        .url(&url_c)
        .open()?;

    input_ctx.dump(0, &output_c)?;

    let input_streams_data: Vec<(AVCodecParameters, AVRational)> = {
        let s = input_ctx.streams();
        info!("Input has {} streams", s.len());
        for (i, stream) in s.iter().enumerate() {
            let codec = stream.codecpar();
            info!("Stream {}: codec_type={:?}, codec_id={}", i, codec.codec_type, codec.codec_id);
        }
        s.iter().map(|st| (st.codecpar().clone(), st.time_base)).collect()
    };

    let format_config = crate::format::FormatConfig {
        format: OutputFormat::Mp4,
        segment_record: false,
        segment_time: 1800,
        video_bitrate: None,
        movflags: None,
        mpegts_flags: None,
    };

    let format_builder = crate::format::FormatBuilder::new(format_config);
    let mut output_ctx = format_builder.build_output_context(&output_c, &input_streams_data)?;

    let mut opts = None;
    output_ctx.write_header(&mut opts)?;

    let mut packet_count: u64 = 0;

    loop {
        match input_ctx.read_packet() {
            Ok(Some(mut packet)) => {
                packet_count += 1;
                let stream_index = packet.stream_index as usize;
                if stream_index < output_ctx.streams().len() {
                    output_ctx.interleaved_write_frame(&mut packet)?;
                }
                if packet_count >= 1000 {
                    info!("Demo mode: capped at 1000 packets");
                    break;
                }
            }
            Ok(None) => {
                info!("EOF after {} packets", packet_count);
                break;
            }
            Err(e) => {
                error!("Read error: {}", e);
                break;
            }
        }
    }

    output_ctx.write_trailer()?;
    info!("Demo completed: wrote {} packets to {}", packet_count, output);

    Ok(())
}

fn generate_default_output(base_dir: &str, _url: &str, format: &OutputFormat) -> String {
    let timestamp = time::OffsetDateTime::now_local()
        .unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let time_str = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        timestamp.year(),
        timestamp.month() as u8,
        timestamp.day(),
        timestamp.hour(),
        timestamp.minute(),
        timestamp.second(),
    );

    let filename = format!("record_{}.{}", time_str, format.file_extension());
    PathBuf::from(base_dir).join(filename).to_string_lossy().to_string()
}

/// 启动 HTTP API 服务并阻塞
async fn cmd_serve(
    addr: &str,
    video_dir: &str,
    check_interval: u64,
) -> anyhow::Result<()> {
    use tokio::sync::RwLock;

    let addr: std::net::SocketAddr = addr.parse()?;
    let video_dir = PathBuf::from(video_dir);
    std::fs::create_dir_all(&video_dir)?;

    // 加载已有配置和录制任务
    let config_mgr = ConfigManager::new(&PathBuf::from(".")).await?;
    let settings = config_mgr.load_settings().await;
    let recordings = config_mgr.load_recordings().await?;

    // 启动 i18n
    if settings.language.starts_with("en") {
        crate::i18n::set_language(crate::i18n::Language::English);
    }

    let mgr = Arc::new(RwLock::new(crate::recording::RecordingManager::new()));

    // 加载已有录制任务到 manager
    {
        let mgr_read = mgr.read().await;
        let formatted: Vec<crate::models::RecordingFormatted> = recordings
            .into_iter()
            .map(|rec| crate::models::RecordingFormatted {
                base: rec,
                status: crate::models::RecordingStatus::Monitoring,
                status_info: crate::models::RecordingStatus::Monitoring.description().to_string(),
                ..Default::default()
            })
            .collect();
        mgr_read.add_recordings(formatted).await;
        mgr_read.set_post_processing(settings.convert_to_mp4, settings.delete_original).await;
    }

    // 启动周期性直播检测
    {
        let mgr_read = mgr.read().await;
        mgr_read.start_periodic_check(check_interval).await;
    }

    // 磁盘空间策略
    let disk_policy = crate::recording::DiskSpacePolicy {
        min_free_gb: settings.recording_space_threshold,
        check_interval_secs: 60,
    };
    info!("Disk policy: stop all recordings when free < {} GB", disk_policy.min_free_gb);

    info!(
        "Platform max concurrent: {}",
        settings.platform_max_concurrent_requests
    );
    let _lang = crate::utils::get_query_params("lang=zh_CN", "lang");

    // 初始化 RecordingController 用于高级编排
    let controller = RecordingController::new(video_dir.to_str().unwrap_or("."));
    controller.set_disk_policy(disk_policy).await;
    let stats = controller.get_stats().await;
    info!("Controller initialized with {} total recordings", stats.total_recordings);

    info!("Starting HTTP server on {}, video dir: {:?}", addr, video_dir);
    crate::web_api::serve(addr, mgr, video_dir).await?;
    Ok(())
}
