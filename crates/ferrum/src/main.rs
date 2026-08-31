mod cli;

use clap::Parser;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> anyhow::Result<()> {
    let args = cli::Cli::parse();
    match args.command {
        cli::Command::Version => {
            println!("{}", cli::version_line());
            Ok(())
        }
        cli::Command::Serve { .. } => {
            anyhow::bail!("serve is not implemented yet")
        }
    }
}
