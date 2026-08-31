use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub build_id: String,
    pub commit_sha: String,
    pub os: String,
    pub arch: String,
}

pub async fn get() -> Json<VersionInfo> {
    let host = ferrum_platform::detect().ok();
    Json(VersionInfo {
        version: crate::cli::VERSION.to_string(),
        build_id: crate::cli::BUILD_ID.to_string(),
        commit_sha: crate::cli::COMMIT_SHA.to_string(),
        os: host
            .as_ref()
            .map(|h| h.pretty_name.clone())
            .unwrap_or_else(|| "unknown".into()),
        arch: std::env::consts::ARCH.to_string(),
    })
}
