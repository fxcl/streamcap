pub mod handlers;
pub mod models;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    routing::{get, post},
};

use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::recording::RecordingManager;

use handlers::*;
use models::AppState;

/// 启动 HTTP 服务 (对应 Python video_stream_service.py / BackendServices API)
pub async fn serve(
    addr: SocketAddr,
    recording_manager: Arc<RwLock<RecordingManager>>,
    video_dir: PathBuf,
) -> anyhow::Result<()> {
    let state = Arc::new(AppState {
        recording_manager,
        video_dir: video_dir.clone(),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // 录制任务 CRUD
        .route("/api/recordings", get(list_recordings).post(add_recording))
        .route(
            "/api/recordings/:id",
            get(get_recording).put(update_recording).delete(delete_recording),
        )
        .route("/api/recordings/:id/start", post(start_recording))
        .route("/api/recordings/:id/stop", post(stop_recording))
        .route("/api/recordings/:id/monitor", post(toggle_monitor))
        // 批量操作
        .route("/api/recordings/start_all", post(start_all))
        .route("/api/recordings/stop_all", post(stop_all))
        // 系统状态
        .route("/api/stats", get(get_stats))
        // 视频文件服务 (对齐 Python video_stream_service.py)
        .route("/api/videos/*path", get(serve_video))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("HTTP server listening on http://{}", addr);
    info!("Video files served from: {:?}", video_dir);

    axum::serve(listener, app).await?;

    Ok(())
}
