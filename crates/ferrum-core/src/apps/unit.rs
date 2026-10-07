use super::processes::Process;
pub use super::processes::{legacy_unit_name, legacy_unit_path, unit_name, unit_path, unit_prefix};
use super::provision::{app_dir, user_name};
use super::{App, AppError};
use crate::deploy::steps::work_dir;
use crate::runtime::{self, Phase, RuntimeKind, toolchain};
use ferrum_platform::ubuntu::SH;
use std::path::Path;

/// `extra` is the other tool's toolchain when the commands name it, on PATH as at build.
/// The process gets its own port as `PORT`; the siblings' ports come from the env file.
pub fn render_unit(
    app: &App,
    process: &Process,
    toolchain: &Path,
    extra: Option<&Path>,
) -> Result<String, AppError> {
    let start = process
        .start()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(AppError::NoProcess)?;

    let dir = app_dir(&app.slug);
    let user = user_name(&app.slug);
    let unit_name = process.unit_name(&app.slug);
    let root = work_dir(&dir.join("current"), &app.root);
    let workdir = if process.dir.is_empty() {
        root
    } else {
        root.join(&process.dir)
    };
    let mut unit = String::new();
    unit.push_str("[Unit]\n");
    unit.push_str(&format!(
        "Description=Ferrum app {} ({})\n",
        app.slug, process.name
    ));
    unit.push_str("After=network.target\n\n");
    unit.push_str("[Service]\n");
    unit.push_str("Type=simple\n");
    unit.push_str(&format!("User={user}\nGroup={user}\n"));
    unit.push_str(&format!("WorkingDirectory={}\n", workdir.display()));
    unit.push_str(&format!(
        "EnvironmentFile={}\n",
        dir.join("shared/.env").display()
    ));
    for (key, value) in runtime::by_kind(app.runtime).env_for(Phase::Run, toolchain, process.port) {
        match extra {
            Some(extra) if key == "PATH" => unit.push_str(&format!(
                "Environment={key}={}\n",
                toolchain::path_with_extra(&value, extra, app.toolchain)
            )),
            _ => unit.push_str(&format!("Environment={key}={value}\n")),
        }
    }
    if app.toolchain == RuntimeKind::Node {
        unit.push_str(&format!(
            "Environment=COREPACK_HOME={}\n",
            dir.join("shared/cache/corepack").display()
        ));
    }
    if let Some(port) = process.port {
        unit.push_str(&format!("Environment=PORT={port}\n"));
    }
    unit.push_str(&format!("ExecStart={SH} -c '{}'\n", exec_quote(start)));
    unit.push_str("Restart=on-failure\nRestartSec=2\n");
    unit.push_str("KillSignal=SIGTERM\nTimeoutStopSec=30\n");
    unit.push_str(&format!("MemoryMax={}M\n", process.memory_mb));
    unit.push_str(&format!("CPUQuota={}%\n", app.cpu_percent));
    unit.push_str("NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\n");
    unit.push_str(&format!(
        "ReadWritePaths={}\n",
        dir.join("shared").display()
    ));
    unit.push_str(&format!("SyslogIdentifier={unit_name}\n\n"));
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
    use crate::apps::processes::ProcessKind;
    use crate::apps::tests::{app, folder, process, worker};
    use crate::runtime::RuntimeKind;

    #[test]
    fn a_process_starts_under_the_app_s_root_directory() {
        let mut a = app("ledger");
        a.root = "apps/web".into();
        let toolchain = Path::new("/var/lib/ferrum/runtimes/node/22.11.0");
        let u = render_unit(&a, &a.processes[0], toolchain, None).unwrap();
        assert!(
            u.contains("WorkingDirectory=/var/lib/ferrum/apps/ledger/current/apps/web\n"),
            "{u}"
        );
        a.processes[0].dir = "server".into();
        let u = render_unit(&a, &a.processes[0], toolchain, None).unwrap();
        assert!(
            u.contains("WorkingDirectory=/var/lib/ferrum/apps/ledger/current/apps/web/server\n"),
            "{u}"
        );
    }

    #[test]
    fn the_unit_runs_as_the_app_user_from_current_with_the_env_file_its_port_and_the_toolchain_on_path()
     {
        let a = app("ledger");
        let u = render_unit(
            &a,
            &a.processes[0],
            Path::new("/var/lib/ferrum/runtimes/node/22.11.0"),
            None,
        )
        .unwrap();
        for line in [
            "Description=Ferrum app ledger (web)",
            "User=ferrum-ledger",
            "Group=ferrum-ledger",
            "WorkingDirectory=/var/lib/ferrum/apps/ledger/current",
            "EnvironmentFile=/var/lib/ferrum/apps/ledger/shared/.env",
            "Environment=PATH=/var/lib/ferrum/runtimes/node/22.11.0/bin:/usr/local/bin:/usr/bin:/bin",
            "Environment=NODE_ENV=production",
            "Environment=PORT=20000",
            "ExecStart=/bin/sh -c 'bun run start'",
            "Restart=on-failure",
            "MemoryMax=512M",
            "CPUQuota=100%",
            "NoNewPrivileges=yes",
            "ProtectSystem=strict",
            "ReadWritePaths=/var/lib/ferrum/apps/ledger/shared",
            "PrivateTmp=yes",
            "SyslogIdentifier=ferrum-app-ledger-web",
            "WantedBy=multi-user.target",
        ] {
            assert!(u.contains(&format!("{line}\n")), "missing {line}\n{u}");
        }
    }

    #[test]
    fn a_process_starts_in_its_own_folder_with_its_own_limit_and_a_worker_gets_no_port() {
        let mut a = app("skool");
        let mut realtime = process("realtime", 20001);
        realtime.dir = "apps/realtime".into();
        realtime.memory_mb = 256;
        let jobs = worker("jobs", "bun run start");
        a.processes = vec![a.processes[0].clone(), realtime, jobs];
        let rt = render_unit(&a, &a.processes[1], Path::new("/t"), None).unwrap();
        assert!(rt.contains("WorkingDirectory=/var/lib/ferrum/apps/skool/current/apps/realtime\n"));
        assert!(rt.contains("Environment=PORT=20001\n"));
        assert!(rt.contains("MemoryMax=256M\n"));
        assert!(rt.contains("SyslogIdentifier=ferrum-app-skool-realtime\n"));
        let j = render_unit(&a, &a.processes[2], Path::new("/t"), None).unwrap();
        assert!(!j.contains("PORT="), "{j}");
        assert!(j.contains("Description=Ferrum app skool (jobs)\n"));
    }

    #[test]
    fn the_other_tool_s_toolchain_is_on_path_with_node_always_before_bun() {
        let a = app("ledger");
        let u = render_unit(
            &a,
            &a.processes[0],
            Path::new("/var/lib/ferrum/runtimes/node/22.11.0"),
            Some(Path::new("/var/lib/ferrum/runtimes/bun/1.2.3")),
        )
        .unwrap();
        assert!(u.contains(
            "Environment=PATH=/var/lib/ferrum/runtimes/node/22.11.0/bin:/var/lib/ferrum/runtimes/bun/1.2.3:/usr/local/bin:/usr/bin:/bin\n"
        ), "{u}");

        let mut b = app("ledger");
        b.runtime = RuntimeKind::Bun;
        b.toolchain = RuntimeKind::Bun;
        let u = render_unit(
            &b,
            &b.processes[0],
            Path::new("/var/lib/ferrum/runtimes/bun/1.2.3"),
            Some(Path::new("/var/lib/ferrum/runtimes/node/22.11.0/bin")),
        )
        .unwrap();
        assert!(u.contains(
            "Environment=PATH=/var/lib/ferrum/runtimes/node/22.11.0/bin:/var/lib/ferrum/runtimes/bun/1.2.3:/usr/local/bin:/usr/bin:/bin\n"
        ), "{u}");
    }

    #[test]
    fn a_folder_process_has_no_unit() {
        let a = app("docs");
        assert!(matches!(
            render_unit(&a, &folder("web", "dist"), Path::new("/x"), None),
            Err(AppError::NoProcess)
        ));
    }

    #[test]
    fn a_dotnet_unit_binds_kestrel_to_its_port() {
        let mut a = app("api");
        a.runtime = RuntimeKind::Dotnet;
        a.toolchain = RuntimeKind::Dotnet;
        a.runtime_version = "9.0".into();
        a.processes[0].kind = ProcessKind::Command {
            start: "dotnet out/Api.dll".into(),
        };
        let u = render_unit(
            &a,
            &a.processes[0],
            Path::new("/var/lib/ferrum/runtimes/dotnet/9.0"),
            None,
        )
        .unwrap();
        assert!(u.contains("Environment=ASPNETCORE_URLS=http://127.0.0.1:20000\n"));
        assert!(u.contains("Environment=DOTNET_ROOT=/var/lib/ferrum/runtimes/dotnet/9.0\n"));
    }

    #[test]
    fn the_start_command_reaches_the_shell_intact() {
        let mut a = app("x");
        a.processes[0].kind = ProcessKind::Command {
            start: "node -e 'console.log(\"$PORT\")' && echo 100%".into(),
        };
        let u = render_unit(&a, &a.processes[0], Path::new("/t"), None).unwrap();
        assert!(
            u.contains(r#"ExecStart=/bin/sh -c 'node -e \'console.log("$$PORT")\' && echo 100%%'"#),
            "{u}"
        );
    }
}
