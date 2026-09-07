use super::App;
use crate::detect::valid_package;
use crate::state::State;
use ferrum_platform::{Platform, PlatformError};
use serde::Serialize;

pub fn parse_aptfile(text: &str) -> (Vec<String>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if valid_package(line) {
            ok.push(line.to_string());
        } else {
            bad.push(line.to_string());
        }
    }
    (ok, bad)
}

fn resolve(platform: &dyn Platform, names: &[String]) -> Vec<String> {
    names
        .iter()
        .flat_map(|n| platform.resolve_package(n))
        .collect()
}

/// Installs `names` and returns the ones the host already had, which are never removed.
pub fn install(platform: &dyn Platform, names: &[String]) -> Result<Vec<String>, PlatformError> {
    let resolved = resolve(platform, names);
    if resolved.is_empty() {
        return Ok(Vec::new());
    }
    let refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let present = platform.installed_packages(&refs)?;
    platform.install_packages(&refs)?;
    Ok(names
        .iter()
        .filter(|n| {
            platform
                .resolve_package(n)
                .iter()
                .all(|p| present.contains(p))
        })
        .cloned()
        .collect())
}

/// Remembers which of `names` Ferrum put on the host; `preexisting` were there before.
pub async fn record(state: &State, names: &[String], preexisting: &[String]) -> anyhow::Result<()> {
    for name in names.iter().filter(|n| !preexisting.contains(n)) {
        sqlx::query!(
            "INSERT OR IGNORE INTO host_packages (name) VALUES (?)",
            name
        )
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

pub async fn add(state: &State, app_id: &str, names: &[String]) -> anyhow::Result<()> {
    for name in names {
        sqlx::query!(
            "INSERT OR IGNORE INTO app_packages (app_id, name) VALUES (?, ?)",
            app_id,
            name
        )
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Removal {
    pub removable: Vec<String>,
    pub kept: Vec<Kept>,
    pub preexisting: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Kept {
    pub name: String,
    pub by: String,
}

/// What dropping `names` from the app would do: kept when another app lists the package,
/// left alone when the host had it before Ferrum, removable otherwise.
pub async fn removal(state: &State, app: &App, names: &[String]) -> anyhow::Result<Removal> {
    let mut out = Removal::default();
    for name in names {
        let other = sqlx::query_scalar!(
            r#"SELECT a.slug AS "slug!" FROM app_packages p JOIN apps a ON a.id = p.app_id
               WHERE p.name = ? AND p.app_id != ? ORDER BY a.slug LIMIT 1"#,
            name,
            app.id
        )
        .fetch_optional(&state.pool)
        .await?;
        if let Some(by) = other {
            out.kept.push(Kept {
                name: name.clone(),
                by,
            });
            continue;
        }
        let ours = sqlx::query_scalar!("SELECT name FROM host_packages WHERE name = ?", name)
            .fetch_optional(&state.pool)
            .await?;
        if ours.is_some() {
            out.removable.push(name.clone());
        } else {
            out.preexisting.push(name.clone());
        }
    }
    Ok(out)
}

pub async fn uninstall(
    state: &State,
    platform: &dyn Platform,
    removal: &Removal,
) -> anyhow::Result<()> {
    if removal.removable.is_empty() {
        return Ok(());
    }
    let resolved = resolve(platform, &removal.removable);
    let refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    platform.remove_packages(&refs)?;
    for name in &removal.removable {
        sqlx::query!("DELETE FROM host_packages WHERE name = ?", name)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use ferrum_platform::FakePlatform;

    #[test]
    fn an_aptfile_keeps_comments_and_blanks_out_and_reports_bad_lines() {
        let (ok, bad) = parse_aptfile("# tools\nffmpeg\n\n  libvips42 \nlibvips; rm -rf /\n");
        assert_eq!(ok, vec!["ffmpeg", "libvips42"]);
        assert_eq!(bad, vec!["libvips; rm -rf /"]);
    }

    #[tokio::test]
    async fn a_package_the_host_had_before_is_never_removable_and_a_shared_one_is_kept() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.answer_installed(&["curl"]);
        let ledger = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let names = vec!["ffmpeg".to_string(), "curl".to_string()];
        add(&state, &ledger.id, &names).await.unwrap();
        let preexisting = install(&p, &names).unwrap();
        assert_eq!(preexisting, vec!["curl"]);
        record(&state, &names, &preexisting).await.unwrap();
        assert_eq!(
            removal(&state, &ledger, &names).await.unwrap(),
            Removal {
                removable: vec!["ffmpeg".into()],
                kept: vec![],
                preexisting: vec!["curl".into()],
            }
        );

        let billing = crate::apps::create(&state, new_app("billing", &[("/", "main", false)]))
            .await
            .unwrap();
        add(&state, &billing.id, &["ffmpeg".to_string()])
            .await
            .unwrap();
        let sorted = removal(&state, &ledger, &names).await.unwrap();
        assert_eq!(
            sorted.kept,
            vec![Kept {
                name: "ffmpeg".into(),
                by: "billing".into()
            }]
        );
        assert!(sorted.removable.is_empty());
        uninstall(&state, &p, &sorted).await.unwrap();
        assert!(p.calls_matching("remove_packages").is_empty());

        crate::apps::delete(&state, "ledger").await.unwrap();
        let last = removal(&state, &billing, &["ffmpeg".to_string()])
            .await
            .unwrap();
        assert_eq!(last.removable, vec!["ffmpeg"]);
        uninstall(&state, &p, &last).await.unwrap();
        assert_eq!(
            p.calls_matching("remove_packages"),
            vec!["remove_packages ffmpeg"]
        );
        let again = removal(&state, &billing, &["ffmpeg".to_string()])
            .await
            .unwrap();
        assert_eq!(
            again.preexisting,
            vec!["ffmpeg"],
            "once removed it is forgotten"
        );
    }
}
