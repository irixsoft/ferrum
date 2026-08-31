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
        cli::Command::Doctor => match ferrum_platform::detect() {
            Ok(info) => {
                println!("host    {} ({})", info.pretty_name, info.arch.as_str());
                println!("ferrum  {}", cli::version_line());
                Ok(())
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        cli::Command::Serve { .. } => {
            anyhow::bail!("serve is not implemented yet")
        }
    }
}
