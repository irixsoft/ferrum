use std::process::Command;

fn main() {
    let commit = std::env::var("FERRUM_COMMIT_SHA").ok().unwrap_or_else(|| {
        Command::new("git")
            .args(["rev-parse", "--short=12", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".into())
    });
    let build_id = std::env::var("FERRUM_BUILD_ID").unwrap_or_else(|_| "dev".into());

    println!("cargo:rustc-env=FERRUM_COMMIT_SHA={commit}");
    println!("cargo:rustc-env=FERRUM_BUILD_ID={build_id}");
    println!("cargo:rerun-if-env-changed=FERRUM_BUILD_ID");
    println!("cargo:rerun-if-env-changed=FERRUM_COMMIT_SHA");
}
