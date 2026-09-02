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
    /// Prints the host card the Dashboard shows.
    Status {
        #[arg(long)]
        token: Option<String>,
    },
    /// Prints an application's log; `--follow` streams it until Ctrl-C.
    Logs {
        slug: String,
        /// app, access or error
        #[arg(long, default_value = "app")]
        source: String,
        #[arg(long, short = 'f')]
        follow: bool,
        #[arg(long, short = 'n', default_value_t = ferrum_core::logs::DEFAULT_LINES)]
        lines: u32,
        #[arg(long)]
        token: Option<String>,
    },
    /// Restarts an application's unit and prints its status.
    Restart {
        slug: String,
        #[arg(long)]
        token: Option<String>,
    },
}

/// `--token` first, then `FERRUM_TOKEN`; the daemon mints nothing for the CLI.
pub fn token_from(flag: Option<String>) -> Option<String> {
    flag.or_else(|| std::env::var("FERRUM_TOKEN").ok())
        .filter(|t| !t.trim().is_empty())
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
