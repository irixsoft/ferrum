use crate::apps::App;
use crate::apps::provision::app_dir;
use crate::state::State;
use crate::time;
use ferrum_platform::Platform;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const KEEP: usize = 5;

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub id: String,
    pub app_id: String,
    pub dir: String,
    pub git_ref: String,
    pub commit_sha: String,
    pub commit_message: Option<String>,
    pub built_at: String,
    pub current: bool,
}

pub fn releases_dir(app: &App) -> PathBuf {
    app_dir(&app.slug).join("releases")
}

pub fn current_link(app: &App) -> PathBuf {
    app_dir(&app.slug).join("current")
}

pub fn release_dir(app: &App, commit_sha: &str) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    releases_dir(app).join(format!("{stamp}_{}", super::short(commit_sha)))
}

pub fn on_disk(platform: &dyn Platform, dir: &Path) -> bool {
    platform.file_exists(&dir.join(".git/HEAD"))
}

pub async fn record(
    state: &State,
    app: &App,
    dir: &Path,
    git_ref: &str,
    commit_sha: &str,
    commit_message: Option<&str>,
) -> anyhow::Result<Release> {
    let id = uuid::Uuid::new_v4().to_string();
    let dir = dir.to_string_lossy().to_string();
    sqlx::query!(
        "INSERT INTO releases (id, app_id, dir, git_ref, commit_sha, commit_message) VALUES (?, ?, ?, ?, ?, ?)",
        id,
        app.id,
        dir,
        git_ref,
        commit_sha,
        commit_message
    )
    .execute(&state.pool)
    .await?;
    by_id(state, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the release vanished as it was recorded"))
}

pub async fn set_current(
    state: &State,
    app_id: &str,
    release_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE apps SET current_release_id = ? WHERE id = ?",
        release_id,
        app_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Release>> {
    Ok(all(state, None).await?.into_iter().find(|r| r.id == id))
}

/// Newest first.
pub async fn for_app(state: &State, app_id: &str) -> anyhow::Result<Vec<Release>> {
    all(state, Some(app_id)).await
}

async fn all(state: &State, app_id: Option<&str>) -> anyhow::Result<Vec<Release>> {
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!", r.app_id AS "app_id!", r.dir AS "dir!", r.git_ref AS "git_ref!",
                  r.commit_sha AS "commit_sha!", r.commit_message, r.built_at AS "built_at!",
                  (a.current_release_id = r.id) AS "current!: bool"
           FROM releases r JOIN apps a ON a.id = r.app_id
           WHERE ? IS NULL OR r.app_id = ?
           ORDER BY r.rowid DESC"#,
        app_id,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Release {
            id: r.id,
            app_id: r.app_id,
            dir: r.dir,
            git_ref: r.git_ref,
            commit_sha: r.commit_sha,
            commit_message: r.commit_message,
            built_at: time::utc(r.built_at),
            current: r.current,
        })
        .collect())
}

/// Removes the oldest releases beyond `keep`, never the current one or any in `protect`.
pub async fn prune(
    state: &State,
    platform: &dyn Platform,
    app: &App,
    keep: usize,
    protect: &[&str],
) -> anyhow::Result<Vec<String>> {
    let releases = for_app(state, &app.id).await?;
    let mut removed = Vec::new();
    for release in releases.iter().skip(keep) {
        if release.current || protect.contains(&release.id.as_str()) {
            continue;
        }
        platform.remove_tree(Path::new(&release.dir))?;
        sqlx::query!("DELETE FROM releases WHERE id = ?", release.id)
            .execute(&state.pool)
            .await?;
        removed.push(release.dir.clone());
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self};
    use ferrum_platform::FakePlatform;

    #[tokio::test]
    async fn releases_are_pruned_to_five_but_never_the_current_or_the_protected() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut ids = Vec::new();
        for i in 0..7 {
            let dir = releases_dir(&app).join(format!("r{i}"));
            p.git_clone("https://github.com/x/y.git", Some("main"), &dir, 1)
                .unwrap();
            let r = record(&state, &app, &dir, "main", &format!("{i}abcdef0"), None)
                .await
                .unwrap();
            ids.push(r.id);
        }
        set_current(&state, &app.id, Some(&ids[0])).await.unwrap();
        let removed = prune(&state, &p, &app, KEEP, &[&ids[1]]).await.unwrap();
        assert!(
            removed.is_empty(),
            "the current and the protected are the two oldest: {removed:?}"
        );

        set_current(&state, &app.id, Some(&ids[6])).await.unwrap();
        let removed = prune(&state, &p, &app, KEEP, &[&ids[1]]).await.unwrap();
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert!(removed[0].ends_with("/r0"));
        let left = for_app(&state, &app.id).await.unwrap();
        assert_eq!(left.len(), 6);
        assert!(left[0].id == ids[6] && left[0].current, "newest first");
        assert!(left.iter().any(|r| r.id == ids[1]));
        assert!(!on_disk(
            &p,
            Path::new("/var/lib/ferrum/apps/ledger/releases/r0")
        ));
        assert!(on_disk(&p, Path::new(&left[0].dir)));
        assert!(
            release_dir(&app, "a3f9c2d4e81b")
                .to_string_lossy()
                .ends_with("_a3f9c2d")
        );
    }
}
