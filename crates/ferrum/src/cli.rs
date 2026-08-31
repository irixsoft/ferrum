use clap::{Parser, Subcommand};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BUILD_ID: &str = env!("FERRUM_BUILD_ID");
pub const COMMIT_SHA: &str = env!("FERRUM_COMMIT_SHA");

#[derive(Parser)]
#[command(name = "ferrum", version = VERSION, disable_version_flag = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Version,
    Doctor,
    Serve {
        #[arg(long, default_value = "/var/lib/ferrum")]
        data_dir: String,
    },
}

pub fn version_line() -> String {
    format!("ferrum {VERSION} (build {BUILD_ID}, commit {COMMIT_SHA})")
}
