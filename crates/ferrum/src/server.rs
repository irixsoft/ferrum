use axum::Router;
use axum::routing::get;
use ferrum_core::state::State;
use std::path::Path;
use tower_http::trace::TraceLayer;

pub use ferrum_core::LISTEN_ADDR;

pub fn app(state: State) -> Router {
    Router::new()
        .route("/api/version", get(crate::routes::version::get))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn serve(data_dir: &Path) -> anyhow::Result<()> {
    let state = State::open(data_dir).await?;
    let listener = tokio::net::TcpListener::bind(LISTEN_ADDR).await?;
    tracing::info!(addr = %LISTEN_ADDR, "listening");
    axum::serve(listener, app(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    let term = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
