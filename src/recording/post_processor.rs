#![allow(dead_code)]
//! Post-processing utilities (TS→MP4 remux, codec conversion).


use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{error, info};

use super::error::RecordingError;

/// TS → MP4 后置转码器
///
/// 对应 Python _do_converts_mp4 (stream_manager.ts 录制后自动转 mp4)
///
/// 因为 rsmpeg remux 模式下没法进行重编码 (c:a aac),
/// 所以这里通过 subprocess ffmpeg 做转码 (仅在需要 convert_to_mp4 时使用)
pub struct FFmpegPostProcessor;

impl FFmpegPostProcessor {
    /// 将 TS 容器无损转封装到 MP4
    ///
    /// cmd: `ffmpeg -i input.ts -c copy -y output.mp4`
    pub async fn remux_ts_to_mp4(
        input_path: &str,
        output_path: &str,
    ) -> Result<(), RecordingError> {
        info!("Converting TS -> MP4: {} -> {}", input_path, output_path);

        let output = Command::new("ffmpeg")
            .args(["-i", input_path, "-c", "copy", "-y", output_path])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| RecordingError::IoError(e))?;

        if output.status.success() {
            info!("Successfully converted TS -> MP4: {}", output_path);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_trimmed = if stderr.len() > 300 {
                &stderr[stderr.len() - 300..]
            } else {
                &stderr
            };
            error!("TS -> MP4 failed: {}", stderr_trimmed);
            Err(RecordingError::FFmpegError(format!(
                "TS to MP4 conversion failed: {}",
                stderr_trimmed
            )))
        }
    }

    /// 将任意容器转封装为指定格式 (音频转 aac + 视频 copy)
    ///
    /// cmd: `ffmpeg -i input -map 0 -c:v copy -c:a aac -f mp4 -movflags +faststart output.mp4`
    pub async fn convert_to_mp4_aac(
        input_path: &str,
        output_path: &str,
    ) -> Result<(), RecordingError> {
        info!("Converting to MP4 with AAC audio: {} -> {}", input_path, output_path);

        let output = Command::new("ffmpeg")
            .args([
                "-i", input_path,
                "-map", "0",
                "-c:v", "copy",
                "-c:a", "aac",
                "-f", "mp4",
                "-movflags", "+faststart+frag_keyframe+empty_moov+delay_moov",
                "-y", output_path,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| RecordingError::IoError(e))?;

        if output.status.success() {
            info!("Successfully converted to MP4 with AAC: {}", output_path);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Conversion failed: {}", stderr);
            Err(RecordingError::FFmpegError(format!(
                "Conversion failed: {}",
                stderr
            )))
        }
    }

    /// 删除原始文件 (对应 Python delete_original 参数)
    pub async fn delete_original(path: &str) -> Result<(), RecordingError> {
        if Path::new(path).exists() {
            tokio::fs::remove_file(path).await?;
            info!("Deleted original file: {}", path);
        }
        Ok(())
    }

    /// 批量查找匹配前缀的分段文件
    ///
    /// 对应 Python: `utils.get_file_paths(os.path.dirname(save_file_path))` + 前缀过滤
    pub fn find_segment_files(dir: &str, prefix: &str) -> Vec<PathBuf> {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with(prefix) && name != prefix {
                        results.push(entry.path());
                    }
                }
            }
        }
        results.sort();
        results
    }

    /// 获取目录中的文件路径列表
    pub fn get_file_paths(dir: &str) -> Vec<PathBuf> {
        let mut results = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    results.push(path);
                }
            }
        }
        results.sort();
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_segment_files_mock() {
        // 不调用实际 IO 的空测试 — 实际使用时要 mock
        let files = FFmpegPostProcessor::find_segment_files("/nonexistent", "test");
        assert!(files.is_empty());
    }

    #[test]
    fn test_get_file_paths_mock() {
        let files = FFmpegPostProcessor::get_file_paths("/nonexistent");
        assert!(files.is_empty());
    }
}
