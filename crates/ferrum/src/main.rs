use clap::Parser;
use ferrum::cli;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FERRUM_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    match cli::Cli::parse().command {
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
        cli::Command::Serve { data_dir } => {
            ferrum::server::serve(std::path::Path::new(&data_dir)).await
        }
        cli::Command::Setup {
            data_dir,
            non_interactive,
            hostname,
            email,
            create_swap,
            staging,
        } => {
            let opts = ferrum::setup::SetupOpts {
                data_dir: data_dir.into(),
                non_interactive,
                hostname,
                email,
                create_swap,
                staging,
            };
            if let Err(e) = ferrum::setup::run(opts).await {
                eprintln!("\n  {e:#}\n");
                std::process::exit(1);
            }
            Ok(())
        }
    }
}
