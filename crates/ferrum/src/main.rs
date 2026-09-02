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
                .unwrap_or_else(|_| "info".into())
                .add_directive("rmcp::service=warn".parse()?),
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
        cli::Command::Passkey {
            command: cli::PasskeyCommand::Enroll { data_dir, user },
        } => {
            let url = report(
                ferrum::admin::enrollment_link(std::path::Path::new(&data_dir), user.as_deref())
                    .await,
            );
            println!("\n  Create a passkey:\n");
            println!("      {url}\n");
            println!(
                "  This link is single-use and expires in {} minutes.\n",
                ferrum_core::enrollment::TTL_MINUTES
            );
            Ok(())
        }
        cli::Command::Token {
            command:
                cli::TokenCommand::Create {
                    data_dir,
                    name,
                    read_only,
                },
        } => {
            let secret = report(
                ferrum::admin::mint_token(std::path::Path::new(&data_dir), &name, read_only).await,
            );
            let access = if read_only { "read-only" } else { "read-write" };
            println!("\n  API token \"{name}\" ({access}):\n");
            println!("      {secret}\n");
            println!("  This is the only time it is shown.\n");
            Ok(())
        }
        cli::Command::Deploy {
            slug,
            git_ref,
            token,
        } => {
            let token = need_token(token);
            let code = report(ferrum::client::deploy(&slug, git_ref.as_deref(), &token).await);
            std::process::exit(code);
        }
        cli::Command::Status { token } => {
            let token = need_token(token);
            report(ferrum::client::status(&token).await);
            Ok(())
        }
        cli::Command::Logs {
            slug,
            source,
            follow,
            lines,
            token,
        } => {
            let token = need_token(token);
            report(ferrum::client::logs(&slug, &source, follow, lines, &token).await);
            Ok(())
        }
        cli::Command::Restart { slug, token } => {
            let token = need_token(token);
            report(ferrum::client::restart(&slug, &token).await);
            Ok(())
        }
    }
}

fn need_token(flag: Option<String>) -> String {
    match cli::token_from(flag) {
        Some(token) => token,
        None => {
            eprintln!(
                "\n  Set FERRUM_TOKEN or pass --token; mint one with `ferrum token create`.\n"
            );
            std::process::exit(2);
        }
    }
}

fn report<T>(result: anyhow::Result<T>) -> T {
    match result {
        Ok(value) => value,
        Err(e) => {
            eprintln!("\n  {e:#}\n");
            std::process::exit(1);
        }
    }
}
