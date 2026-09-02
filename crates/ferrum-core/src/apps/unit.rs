use super::provision::{app_dir, user_name};
use super::{App, AppError};
use crate::runtime::{self, Phase};
use ferrum_platform::ubuntu::{SH, SYSTEMD_UNIT_DIR};
use std::path::{Path, PathBuf};

pub fn unit_name(slug: &str) -> String {
    format!("ferrum-app-{slug}")
}

pub fn unit_path(slug: &str) -> PathBuf {
    Path::new(SYSTEMD_UNIT_DIR).join(format!("{}.service", unit_name(slug)))
}

pub fn render_unit(app: &App, toolchain: &Path) -> Result<String, AppError> {
    if !app.runtime.has_process() {
        return Err(AppError::NoProcess);
    }
    let start = app
        .commands
        .start
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or(AppError::NoProcess)?;

    let dir = app_dir(&app.slug);
    let user = user_name(&app.slug);
    let mut unit = String::new();
    unit.push_str("[Unit]\n");
    unit.push_str(&format!("Description=Ferrum app {}\n", app.slug));
    unit.push_str("After=network.target\n\n");
    unit.push_str("[Service]\n");
    unit.push_str("Type=simple\n");
    unit.push_str(&format!("User={user}\nGroup={user}\n"));
    unit.push_str(&format!(
        "WorkingDirectory={}\n",
        dir.join("current").display()
    ));
    unit.push_str(&format!(
        "EnvironmentFile={}\n",
        dir.join("shared/.env").display()
    ));
    for (key, value) in
        runtime::by_kind(app.runtime).env_for(Phase::Run, toolchain, app.main_port())
    {
        unit.push_str(&format!("Environment={key}={value}\n"));
    }
    unit.push_str(&format!("ExecStart={SH} -c '{}'\n", exec_quote(start)));
    unit.push_str("Restart=on-failure\nRestartSec=2\n");
    unit.push_str("KillSignal=SIGTERM\nTimeoutStopSec=30\n");
    unit.push_str(&format!("MemoryMax={}M\n", app.memory_mb));
    unit.push_str(&format!("CPUQuota={}%\n", app.cpu_percent));
    unit.push_str("NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\n");
    unit.push_str(&format!(
        "ReadWritePaths={}\n",
        dir.join("shared").display()
    ));
    unit.push_str(&format!("SyslogIdentifier={}\n\n", unit_name(&app.slug)));
    unit.push_str("[Install]\nWantedBy=multi-user.target\n");
    Ok(unit)
}

/// Inside systemd's single quotes a backslash still escapes, and `$` would be expanded by
/// systemd before the shell ever saw it.
fn exec_quote(command: &str) -> String {
    let mut out = String::with_capacity(command.len());
    for c in command.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '$' => out.push_str("$$"),
            '%' => out.push_str("%%"),
            '\n' => out.push(' '),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::app;
    use crate::runtime::RuntimeKind;

    #[test]
    fn the_unit_runs_as_the_app_user_from_current_with_the_env_file_and_the_toolchain_on_path() {
        let u = render_unit(
            &app("ledger"),
            Path::new("/var/lib/ferrum/runtimes/node/22.11.0"),
        )
        .unwrap();
        for line in [
            "User=ferrum-ledger",
            "Group=ferrum-ledger",
            "WorkingDirectory=/var/lib/ferrum/apps/ledger/current",
            "EnvironmentFile=/var/lib/ferrum/apps/ledger/shared/.env",
            "Environment=PATH=/var/lib/ferrum/runtimes/node/22.11.0/bin:/usr/local/bin:/usr/bin:/bin",
            "Environment=NODE_ENV=production",
            "ExecStart=/bin/sh -c 'bun run start'",
            "Restart=on-failure",
            "MemoryMax=512M",
            "CPUQuota=100%",
            "NoNewPrivileges=yes",
            "ProtectSystem=strict",
            "ReadWritePaths=/var/lib/ferrum/apps/ledger/shared",
            "PrivateTmp=yes",
            "WantedBy=multi-user.target",
        ] {
            assert!(u.contains(&format!("{line}\n")), "missing {line}\n{u}");
        }
        assert!(
            !u.contains("PORT="),
            "ports come from the env file, not the unit"
        );
    }

    #[test]
    fn a_static_app_has_no_unit() {
        let mut a = app("docs");
        a.runtime = RuntimeKind::Static;
        assert!(matches!(
            render_unit(&a, Path::new("/x")),
            Err(AppError::NoProcess)
        ));
    }

    #[test]
    fn a_dotnet_unit_binds_kestrel_to_its_port() {
        let mut a = app("api");
        a.runtime = RuntimeKind::Dotnet;
        a.toolchain = RuntimeKind::Dotnet;
        a.runtime_version = "9.0".into();
        a.commands.start = Some("dotnet out/Api.dll".into());
        let u = render_unit(&a, Path::new("/var/lib/ferrum/runtimes/dotnet/9.0")).unwrap();
        assert!(u.contains("Environment=ASPNETCORE_URLS=http://127.0.0.1:20000\n"));
        assert!(u.contains("Environment=DOTNET_ROOT=/var/lib/ferrum/runtimes/dotnet/9.0\n"));
    }

    #[test]
    fn the_start_command_reaches_the_shell_intact() {
        let mut a = app("x");
        a.commands.start = Some("node -e 'console.log(\"$PORT\")' && echo 100%".into());
        let u = render_unit(&a, Path::new("/t")).unwrap();
        assert!(
            u.contains(r#"ExecStart=/bin/sh -c 'node -e \'console.log("$$PORT")\' && echo 100%%'"#),
            "{u}"
        );
    }

    #[test]
    fn unit_names_and_paths_follow_the_slug() {
        assert_eq!(unit_name("ledger"), "ferrum-app-ledger");
        assert_eq!(
            unit_path("ledger"),
            Path::new("/etc/systemd/system/ferrum-app-ledger.service")
        );
    }
}
