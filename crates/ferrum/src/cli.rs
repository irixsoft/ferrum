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
        #[arg(long, default_value = ferrum_core::DATA_DIR)]
        data_dir: String,
    },
    Setup {
        #[arg(long, default_value = ferrum_core::DATA_DIR)]
        data_dir: String,
        #[arg(long)]
        non_interactive: bool,
        #[arg(long)]
        hostname: Option<String>,
        #[arg(long)]
        email: Option<String>,
        #[arg(long)]
        create_swap: Option<bool>,
        #[arg(long)]
        staging: bool,
    },
    Passkey {
        #[command(subcommand)]
        command: PasskeyCommand,
    },
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Queues a deploy through the running daemon and follows its log.
    Deploy {
        slug: String,
        #[arg(long = "ref")]
        git_ref: Option<String>,
        /// Defaults to FERRUM_TOKEN; mint one with `ferrum token create`.
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum PasskeyCommand {
    Enroll {
        #[arg(long, default_value = ferrum_core::DATA_DIR)]
        data_dir: String,
        #[arg(long)]
        user: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TokenCommand {
    Create {
        #[arg(long, default_value = ferrum_core::DATA_DIR)]
        data_dir: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        read_only: bool,
    },
}

pub fn version_line() -> String {
    format!("ferrum {VERSION} (build {BUILD_ID}, commit {COMMIT_SHA})")
}
