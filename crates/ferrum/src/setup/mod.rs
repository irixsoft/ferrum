pub mod prompt;

use anyhow::{Context, bail};
use ferrum_core::acme::{self, Directory, Issuer};
use ferrum_core::dns::{self, Verdict};
use ferrum_core::setup::{self, Stage};
use ferrum_core::state::State;
use ferrum_core::{FERRUM_UNIT, enrollment, nginx, swap, users};
use ferrum_platform::{Platform, ServiceAction, Ubuntu};
use std::net::IpAddr;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const DNS_TIMEOUT_INTERACTIVE: Duration = Duration::from_secs(60 * 60);
const DNS_TIMEOUT_UNATTENDED: Duration = Duration::from_secs(30 * 60);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct SetupOpts {
    pub data_dir: PathBuf,
    pub non_interactive: bool,
    pub hostname: Option<String>,
    pub email: Option<String>,
    pub create_swap: Option<bool>,
    pub staging: bool,
}

pub async fn run(opts: SetupOpts) -> anyhow::Result<()> {
    let host_info = ferrum_platform::detect().map_err(|e| anyhow::anyhow!("{e}"))?;
    require_root()?;

    let platform = Ubuntu;
    let state = State::open(&opts.data_dir).await?;
    let stage = setup::stage(&state).await?;

    println!("\n  Ferrum setup — {}\n", host_info.pretty_name);

    let hostname = resolve_hostname(&state, &opts).await?;
    let email = resolve_email(&state, &opts).await?;
    let swap_mb = decide_swap(&platform, &opts)?;
    setup::advance(&state, Stage::HostnameSet).await?;

    let public_ip = dns::public_ip()
        .await
        .context("determining this server's public IP address")?;

    println!("\n  Add this DNS record:\n");
    println!("      {hostname}    A    {public_ip}\n");
    println!("  Ferrum will install the platform while you do.\n");

    let codename = host_info.codename.clone();
    let already_installed = stage >= Stage::PlatformInstalled;
    let install = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let platform = Ubuntu;
        if let Some(mb) = swap_mb {
            swap::create(&platform, mb)?;
        }
        if !already_installed {
            nginx::install(&platform, &codename)?;
        }
        Ok(())
    });

    let timeout = if opts.non_interactive {
        DNS_TIMEOUT_UNATTENDED
    } else {
        DNS_TIMEOUT_INTERACTIVE
    };

    let (installed, resolved) = tokio::join!(install, wait_for_dns(&hostname, public_ip, timeout));

    installed.context("the platform install task failed")??;
    if !already_installed {
        setup::advance(&state, Stage::PlatformInstalled).await?;
        println!("  Platform installed.");
    }
    resolved?;

    let cert_dir = acme::cert_dir(&hostname);
    if stage < Stage::CertIssued || !cert_dir.join("fullchain.pem").exists() {
        println!("  Requesting a certificate for {hostname}…");
        let directory = if opts.staging {
            Directory::Staging
        } else {
            Directory::LetsEncrypt
        };
        crate::server::set_acme_directory(&state, opts.staging).await?;
        let issuer = Issuer::new(&state, directory, &email).await?;
        let cert = issuer.issue(&hostname, public_ip, &cert_dir).await?;
        setup::advance(&state, Stage::CertIssued).await?;
        println!(
            "  Certificate issued, valid until {}.",
            cert.not_after.date()
        );
    } else {
        println!("  Certificate already issued; keeping it.");
    }

    let vhost = nginx::render_panel_vhost(&hostname, &cert_dir);
    nginx::write_and_reload(&platform, &nginx::panel_conf_path(), &vhost)?;

    platform.service(ServiceAction::EnableNow, FERRUM_UNIT)?;
    wait_for_health().await?;
    setup::advance(&state, Stage::Complete).await?;

    println!("\n  Ferrum is running at https://{hostname}\n");

    if users::count(&state).await? == 0 {
        let user = users::create(&state, &email).await?;
        let token = enrollment::issue(&state, &user.id).await?;
        println!("  Create your passkey:\n");
        println!("      {}\n", enrollment::url(&hostname, &token));
        println!(
            "  This link is single-use and expires in {} minutes.",
            enrollment::TTL_MINUTES
        );
        println!("  Run `ferrum passkey enroll` for a new one.\n");
    }

    Ok(())
}

fn require_root() -> anyhow::Result<()> {
    let uid = std::fs::metadata("/proc/self")
        .context("reading /proc/self to determine the current user")?
        .uid();
    if uid != 0 {
        bail!(
            "Setup configures nginx, systemd and swap, so it must run as root. Try: sudo ferrum setup"
        );
    }
    Ok(())
}

async fn resolve_hostname(state: &State, opts: &SetupOpts) -> anyhow::Result<String> {
    if let Some(existing) = setup::hostname(state).await? {
        println!("  Panel hostname: {existing}");
        return Ok(existing);
    }
    let supplied = opts
        .hostname
        .clone()
        .or_else(|| std::env::var("FERRUM_HOSTNAME").ok());

    let hostname = match supplied {
        Some(v) => prompt::validate_hostname(&v).map_err(|e| anyhow::anyhow!("{e}"))?,
        None if opts.non_interactive => {
            bail!("--non-interactive requires --hostname (or FERRUM_HOSTNAME)")
        }
        None => {
            prompt::require_terminal()?;
            prompt::ask(
                "Panel hostname?",
                "e.g. panel.example.com",
                prompt::validate_hostname,
            )?
        }
    };
    setup::set_hostname(state, &hostname).await?;
    Ok(hostname)
}

async fn resolve_email(state: &State, opts: &SetupOpts) -> anyhow::Result<String> {
    if let Some(existing) = setup::email(state).await? {
        return Ok(existing);
    }
    let supplied = opts
        .email
        .clone()
        .or_else(|| std::env::var("FERRUM_ACME_EMAIL").ok());

    let email = match supplied {
        Some(v) => prompt::validate_email(&v).map_err(|e| anyhow::anyhow!("{e}"))?,
        None if opts.non_interactive => {
            bail!("--non-interactive requires --email (or FERRUM_ACME_EMAIL)")
        }
        None => {
            prompt::require_terminal()?;
            prompt::ask(
                "Email for certificate notices?",
                "e.g. you@example.com",
                prompt::validate_email,
            )?
        }
    };
    setup::set_email(state, &email).await?;
    Ok(email)
}

fn decide_swap(platform: &dyn Platform, opts: &SetupOpts) -> anyhow::Result<Option<u64>> {
    if !swap::needs_swap(platform)? {
        return Ok(None);
    }
    let size = swap::recommended_mb(platform.total_memory_kb()?);

    let wanted = match opts.create_swap.or_else(env_flag) {
        Some(v) => v,
        None if opts.non_interactive => true,
        None => {
            println!(
                "\n  This server has no swap. Builds routinely peak above available RAM, and\n  without swap the kernel kills them outright."
            );
            prompt::require_terminal()?;
            prompt::confirm(&format!("Create a {size} MB swapfile?"), true)?
        }
    };
    Ok(wanted.then_some(size))
}

fn env_flag() -> Option<bool> {
    match std::env::var("FERRUM_CREATE_SWAP")
        .ok()?
        .to_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

async fn wait_for_dns(host: &str, expected: IpAddr, timeout: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last = String::new();

    loop {
        match dns::verify(host, expected).await {
            Ok(Verdict::Match) => {
                println!("  {host} now points at {expected}.");
                return Ok(());
            }
            Ok(verdict) => {
                let message = dns::describe(&verdict, host);
                if message != last {
                    println!("  {message}");
                    last = message;
                }
            }
            Err(e) => {
                let message = e.to_string();
                if message != last {
                    println!("  {message}");
                    last = message;
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("{host} did not point at {expected} in time. {last}");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn wait_for_health() -> anyhow::Result<()> {
    let url = format!("http://{}/api/health", ferrum_core::LISTEN_ADDR);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let deadline = Instant::now() + HEALTH_TIMEOUT;

    loop {
        if let Ok(res) = client.get(&url).send().await
            && res.status().is_success()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("The Ferrum service did not become healthy. Check: journalctl -u {FERRUM_UNIT}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
