use axum::Json;
use ferrum_core::host::Build;
use serde::Serialize;

#[derive(Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub build_id: String,
    pub commit_sha: String,
    pub os: String,
    pub arch: String,
}

pub fn build() -> Build {
    let host = ferrum_platform::detect().ok();
    Build {
        version: crate::cli::VERSION.to_string(),
        build_id: crate::cli::BUILD_ID.to_string(),
        commit_sha: crate::cli::COMMIT_SHA.to_string(),
        os: host
            .as_ref()
            .map(|h| h.pretty_name.clone())
            .unwrap_or_else(|| "unknown".into()),
        arch: std::env::consts::ARCH.to_string(),
    }
}

pub async fn get() -> Json<VersionInfo> {
    let build = build();
    Json(VersionInfo {
        version: build.version,
        build_id: build.build_id,
        commit_sha: build.commit_sha,
        os: build.os,
        arch: build.arch,
    })
}
