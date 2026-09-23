use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tracing::info;

use super::error::RecordingError;
use super::remux_engine::RemuxEngine;

/// 直接下载直播流 (HTTP chunked transfer)
pub async fn download_stream(
    url: &str,
    save_path: &str,
    headers: Option<&str>,
    engine: Arc<Mutex<RemuxEngine>>,
) -> Result<u64, RecordingError> {
    info!("Starting direct stream download: {}", url);

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent("Mozilla/5.0 (Linux; Android 11; SAMSUNG SM-G973U) AppleWebKit/537.36")
        .build()
        .map_err(|e| RecordingError::NetworkError(e.to_string()))?;

    let mut request = client.get(url);

    if let Some(hdrs) = headers {
        for line in hdrs.lines() {
            if let Some((key, value)) = line.split_once(':') {
                request = request.header(key.trim(), value.trim());
            }
        }
    }

    let response = request
        .send()
        .await
        .map_err(|e| RecordingError::NetworkError(e.to_string()))?;

    let status = response.status();
    if !status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(RecordingError::NetworkError(
            format!("HTTP request failed with status: {}", status)
        ));
    }

    let mut file = File::create(save_path).await?;
    let mut stream = response.bytes_stream();
    let start_time = Instant::now();
    let mut total_bytes: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        if engine.blocking_lock().stop_signal.load(std::sync::atomic::Ordering::SeqCst) {
            info!("Download stopped by user after {} bytes", total_bytes);
            break;
        }

        let chunk = chunk_result.map_err(|e| RecordingError::NetworkError(e.to_string()))?;
        file.write_all(&chunk).await?;
        total_bytes += chunk.len() as u64;
    }

    file.flush().await?;

    let elapsed = start_time.elapsed().as_secs();
    info!(
        "Direct download completed: {} bytes in {}s",
        total_bytes, elapsed,
    );

    Ok(total_bytes)
}
