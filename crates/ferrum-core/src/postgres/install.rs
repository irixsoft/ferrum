use super::tune::{MAX_CONNECTIONS, render_conf, tuning};
use crate::state::State;
use anyhow::bail;
use ferrum_platform::ubuntu::{PGDG_KEY_URL, pg_cluster_unit, pg_conf_path, pgdg_repo_line};
use ferrum_platform::{Platform, ServiceAction};

pub const DEFAULT_MAJOR: u32 = 18;
const MAJOR_SETTING: &str = "postgres.major";
const REPO_NAME: &str = "pgdg";

pub async fn major(state: &State) -> anyhow::Result<Option<u32>> {
    Ok(state
        .get_setting(MAJOR_SETTING)
        .await?
        .and_then(|v| v.parse().ok()))
}

/// Installs the pinned major (or the default on first use), or adopts a cluster already on the
/// host. A pinned major is never upgraded because a repository moved on.
pub async fn ensure_installed(
    state: &State,
    platform: &dyn Platform,
    codename: &str,
) -> anyhow::Result<u32> {
    let pinned = major(state).await?;
    let present = platform.postgres_major_installed();
    let target = match (pinned, present) {
        (Some(p), Some(h)) if p != h => bail!(
            "PostgreSQL {p} is pinned for this host but PostgreSQL {h} is installed. Ferrum never changes a major version on its own."
        ),
        (Some(p), Some(_)) => {
            if write_conf(platform, p)? {
                platform.service(ServiceAction::Reload, &pg_cluster_unit(p))?;
            }
            return Ok(p);
        }
        (None, Some(h)) => {
            state.set_setting(MAJOR_SETTING, &h.to_string()).await?;
            write_conf(platform, h)?;
            platform.service(ServiceAction::Restart, &pg_cluster_unit(h))?;
            return Ok(h);
        }
        (Some(p), None) => p,
        (None, None) => DEFAULT_MAJOR,
    };

    platform.add_apt_repo(REPO_NAME, PGDG_KEY_URL, &pgdg_repo_line(codename))?;
    platform.install_packages(&[&package(target)])?;
    state
        .set_setting(MAJOR_SETTING, &target.to_string())
        .await?;
    write_conf(platform, target)?;
    platform.service(ServiceAction::Restart, &pg_cluster_unit(target))?;
    Ok(target)
}

pub fn package(major: u32) -> String {
    format!("postgresql-{major}")
}

pub fn extension_package(major: u32, extension: &str) -> String {
    format!("postgresql-{major}-{extension}")
}

fn write_conf(platform: &dyn Platform, major: u32) -> anyhow::Result<bool> {
    let path = pg_conf_path(major);
    let conf = render_conf(&tuning(platform.total_memory_kb()?, MAX_CONNECTIONS));
    if platform.read_file(&path)?.as_deref() == Some(conf.as_str()) {
        return Ok(false);
    }
    platform.write_file(&path, &conf, 0o644)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::tests::state;
    use ferrum_platform::FakePlatform;

    fn position(calls: &[String], needle: &str) -> usize {
        calls
            .iter()
            .position(|c| c == needle)
            .unwrap_or_else(|| panic!("no call {needle:?} in {calls:#?}"))
    }

    #[tokio::test]
    async fn install_adds_pgdg_installs_the_default_major_writes_the_conf_and_restarts() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let major = ensure_installed(&state, &p, "noble").await.unwrap();
        assert_eq!(major, DEFAULT_MAJOR);
        let calls = p.calls();
        let repo = position(
            &calls,
            "add_apt_repo pgdg https://www.postgresql.org/media/keys/ACCC4CF8.asc https://apt.postgresql.org/pub/repos/apt noble-pgdg main",
        );
        let pkg = position(&calls, &format!("install_packages postgresql-{major}"));
        let conf = position(
            &calls,
            &format!("write_file /etc/postgresql/{major}/main/conf.d/ferrum.conf 644"),
        );
        let restart = position(&calls, &format!("service restart postgresql@{major}-main"));
        assert!(repo < pkg && pkg < conf && conf < restart, "{calls:#?}");
        assert_eq!(super::major(&state).await.unwrap(), Some(major));
        let written = p
            .written(&format!("/etc/postgresql/{major}/main/conf.d/ferrum.conf"))
            .unwrap();
        assert!(written.contains("shared_buffers = 512MB\n"), "{written}");
    }

    #[tokio::test]
    async fn a_second_call_reuses_the_pinned_major_and_touches_no_repo() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        ensure_installed(&state, &p, "noble").await.unwrap();
        let p2 = FakePlatform::new();
        p2.set_postgres_major(DEFAULT_MAJOR);
        assert_eq!(
            ensure_installed(&state, &p2, "noble").await.unwrap(),
            DEFAULT_MAJOR
        );
        assert!(p2.calls_matching("add_apt_repo").is_empty());
        assert!(p2.calls_matching("install_packages").is_empty());
        assert!(p2.calls_matching("service restart").is_empty());
        assert!(
            p2.calls_matching("service reload").len() == 1,
            "the conf was missing on this platform, so it was written and reloaded"
        );
        assert!(
            ensure_installed(&state, &p2, "noble").await.is_ok()
                && p2.calls_matching("service reload").len() == 1,
            "an unchanged conf is not rewritten"
        );
    }

    #[tokio::test]
    async fn a_cluster_already_on_the_host_is_adopted_and_pinned() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.set_postgres_major(16);
        assert_eq!(ensure_installed(&state, &p, "noble").await.unwrap(), 16);
        assert_eq!(major(&state).await.unwrap(), Some(16));
        assert!(p.calls_matching("install_packages").is_empty());
        assert!(
            p.calls()
                .contains(&"service restart postgresql@16-main".to_string())
        );
    }

    #[tokio::test]
    async fn a_pinned_major_is_never_upgraded_to_match_the_host() {
        let (_d, state) = state().await;
        state.set_setting(MAJOR_SETTING, "17").await.unwrap();
        let p = FakePlatform::new();
        p.set_postgres_major(18);
        let e = ensure_installed(&state, &p, "noble").await.unwrap_err();
        assert!(
            e.to_string().contains("17") && e.to_string().contains("18"),
            "{e}"
        );
        assert!(p.calls().is_empty());
    }
}
