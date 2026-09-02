pub mod log;
pub mod maintenance;
pub mod queue;
pub mod releases;
pub mod run;
pub mod snapshots;
pub mod steps;

pub use queue::{Deployer, head_of, matches};
pub use run::Ctx;

use crate::apps::App;
use crate::state::State;
use crate::time;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const RESTARTED: &str = "Ferrum restarted while this deploy was running.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
pub enum DeployState {
    Queued,
    Cloning,
    InstallingSystemPackages,
    InstallingDeps,
    Building,
    Snapshotting,
    MaintenanceOn,
    Migrating,
    Swapping,
    Restarting,
    HealthChecking,
    MaintenanceOff,
}

impl DeployState {
    pub const ALL: [DeployState; 12] = [
        Self::Queued,
        Self::Cloning,
        Self::InstallingSystemPackages,
        Self::InstallingDeps,
        Self::Building,
        Self::Snapshotting,
        Self::MaintenanceOn,
        Self::Migrating,
        Self::Swapping,
        Self::Restarting,
        Self::HealthChecking,
        Self::MaintenanceOff,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Cloning => "Cloning",
            Self::InstallingSystemPackages => "InstallingSystemPackages",
            Self::InstallingDeps => "InstallingDeps",
            Self::Building => "Building",
            Self::Snapshotting => "Snapshotting",
            Self::MaintenanceOn => "MaintenanceOn",
            Self::Migrating => "Migrating",
            Self::Swapping => "Swapping",
            Self::Restarting => "Restarting",
            Self::HealthChecking => "HealthChecking",
            Self::MaintenanceOff => "MaintenanceOff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(rename_all = "lowercase")]
pub enum Outcome {
    Live,
    Failed,
    RolledBack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Trigger {
    Webhook,
    Manual,
    Cli,
    Rollback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum StepStatus {
    Done,
    Active,
    Pending,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub state: DeployState,
    pub status: StepStatus,
    pub elapsed_secs: Option<u64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Deploy {
    pub id: String,
    pub app_id: String,
    pub app_slug: String,
    pub trigger: Trigger,
    pub git_ref: String,
    pub commit_sha: Option<String>,
    pub commit_message: Option<String>,
    pub author: Option<String>,
    pub state: Option<DeployState>,
    pub outcome: Option<Outcome>,
    pub failure_reason: Option<String>,
    pub queue_position: Option<u32>,
    pub release_id: Option<String>,
    pub restore_deploy_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub duration_secs: Option<u64>,
    pub steps: Vec<Step>,
    pub snapshots: Vec<snapshots::Snapshot>,
}

impl Deploy {
    pub fn is_running(&self) -> bool {
        self.state.is_some_and(|s| s != DeployState::Queued)
    }

    pub fn short_sha(&self) -> String {
        short(self.commit_sha.as_deref().unwrap_or(&self.git_ref))
    }
}

pub fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// What a deploy is asked to build; a rollback names a release instead.
#[derive(Debug, Clone, Default)]
pub struct Commit {
    pub sha: Option<String>,
    pub message: Option<String>,
    pub author: Option<String>,
}

pub async fn create(
    state: &State,
    app: &App,
    trigger: Trigger,
    git_ref: &str,
    commit: &Commit,
) -> anyhow::Result<Deploy> {
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = state.pool.begin().await?;
    sqlx::query!(
        "INSERT INTO deploys (id, app_id, trigger, git_ref, commit_sha, commit_message, author, state)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        app.id,
        trigger,
        git_ref,
        commit.sha,
        commit.message,
        commit.author,
        DeployState::Queued,
    )
    .execute(&mut *tx)
    .await?;
    for (position, step) in DeployState::ALL.iter().enumerate() {
        let position = position as i64;
        let status = if *step == DeployState::Queued {
            StepStatus::Active
        } else {
            StepStatus::Pending
        };
        sqlx::query!(
            "INSERT INTO deploy_steps (deploy_id, state, position, status, started_at)
             VALUES (?, ?, ?, ?, CASE WHEN ? = 'active' THEN datetime('now') END)",
            id,
            step,
            position,
            status,
            status,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    by_id(state, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the deploy vanished as it was created"))
}

pub async fn set_rollback_target(
    state: &State,
    id: &str,
    release_id: &str,
    restore_deploy_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE deploys SET release_id = ?, restore_deploy_id = ? WHERE id = ?",
        release_id,
        restore_deploy_id,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn retarget(
    state: &State,
    id: &str,
    git_ref: &str,
    commit: &Commit,
) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE deploys SET git_ref = ?, commit_sha = ?, commit_message = ?, author = ? WHERE id = ?",
        git_ref,
        commit.sha,
        commit.message,
        commit.author,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn set_commit(state: &State, id: &str, commit: &Commit) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE deploys SET commit_sha = coalesce(?, commit_sha),
                            commit_message = coalesce(?, commit_message),
                            author = coalesce(?, author)
         WHERE id = ?",
        commit.sha,
        commit.message,
        commit.author,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Marks the active step done and the named one active.
pub async fn enter(state: &State, id: &str, next: DeployState) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    close_active(&mut tx, id, StepStatus::Done).await?;
    sqlx::query!(
        "UPDATE deploy_steps SET status = 'active', started_at = datetime('now'), finished_at = NULL
         WHERE deploy_id = ? AND state = ?",
        id,
        next
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("UPDATE deploys SET state = ? WHERE id = ?", next, id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn skip(state: &State, id: &str, step: DeployState, note: &str) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    close_active(&mut tx, id, StepStatus::Done).await?;
    sqlx::query!(
        "UPDATE deploy_steps SET status = 'skipped', note = ? WHERE deploy_id = ? AND state = ?",
        note,
        id,
        step
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn note(state: &State, id: &str, step: DeployState, note: &str) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE deploy_steps SET note = ? WHERE deploy_id = ? AND state = ?",
        note,
        id,
        step
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn close_active(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &str,
    status: StepStatus,
) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE deploy_steps SET status = ?, finished_at = datetime('now')
         WHERE deploy_id = ? AND status = 'active'",
        status,
        id
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn finish(
    state: &State,
    id: &str,
    outcome: Outcome,
    failure_reason: Option<&str>,
    release_id: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    let status = if outcome == Outcome::Live {
        StepStatus::Done
    } else {
        StepStatus::Failed
    };
    close_active(&mut tx, id, status).await?;
    sqlx::query!(
        "UPDATE deploys SET state = NULL, outcome = ?, failure_reason = ?,
                            release_id = coalesce(?, release_id), finished_at = datetime('now')
         WHERE id = ?",
        outcome,
        failure_reason,
        release_id,
        id
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn abandon_unfinished(state: &State) -> anyhow::Result<u32> {
    let ids = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM deploys WHERE outcome IS NULL"#)
        .fetch_all(&state.pool)
        .await?;
    for id in &ids {
        finish(state, id, Outcome::Failed, Some(RESTARTED), None).await?;
    }
    Ok(ids.len() as u32)
}

pub async fn delete(state: &State, id: &str) -> anyhow::Result<bool> {
    let done = sqlx::query!("DELETE FROM deploys WHERE id = ?", id)
        .execute(&state.pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Deploy>> {
    Ok(fetch(state, Some(id), None, 1).await?.into_iter().next())
}

pub async fn list(state: &State, app_id: Option<&str>, limit: u32) -> anyhow::Result<Vec<Deploy>> {
    fetch(state, None, app_id, limit).await
}

pub async fn running(state: &State) -> anyhow::Result<Option<Deploy>> {
    Ok(fetch(state, None, None, 200)
        .await?
        .into_iter()
        .find(Deploy::is_running))
}

pub async fn running_for(state: &State, app_id: &str) -> anyhow::Result<Option<Deploy>> {
    Ok(running(state).await?.filter(|d| d.app_id == app_id))
}

pub async fn queued_for(state: &State, app_id: &str) -> anyhow::Result<Option<Deploy>> {
    Ok(fetch(state, None, Some(app_id), 200)
        .await?
        .into_iter()
        .find(|d| d.state == Some(DeployState::Queued)))
}

pub async fn latest_for(state: &State, app_id: &str) -> anyhow::Result<Option<Deploy>> {
    Ok(fetch(state, None, Some(app_id), 1)
        .await?
        .into_iter()
        .next())
}

async fn queue_positions(state: &State) -> anyhow::Result<HashMap<String, u32>> {
    let running: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!: i64" FROM deploys WHERE state IS NOT NULL AND state != 'Queued'"#
    )
    .fetch_one(&state.pool)
    .await?;
    let queued = sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM deploys WHERE state = 'Queued' ORDER BY rowid"#
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(queued
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, running as u32 + i as u32))
        .collect())
}

async fn fetch(
    state: &State,
    id: Option<&str>,
    app_id: Option<&str>,
    limit: u32,
) -> anyhow::Result<Vec<Deploy>> {
    let limit = limit as i64;
    let rows = sqlx::query!(
        r#"SELECT d.id AS "id!", d.app_id AS "app_id!", a.slug AS "app_slug!",
                  d.trigger AS "trigger!: Trigger", d.git_ref AS "git_ref!", d.commit_sha,
                  d.commit_message, d.author, d.state AS "state: DeployState",
                  d.outcome AS "outcome: Outcome", d.failure_reason, d.release_id,
                  d.restore_deploy_id, d.started_at AS "started_at!", d.finished_at,
                  CAST(strftime('%s', d.finished_at) - strftime('%s', d.started_at) AS INTEGER) AS "duration_secs: i64"
           FROM deploys d JOIN apps a ON a.id = d.app_id
           WHERE (? IS NULL OR d.id = ?) AND (? IS NULL OR d.app_id = ?)
           ORDER BY d.rowid DESC LIMIT ?"#,
        id,
        id,
        app_id,
        app_id,
        limit
    )
    .fetch_all(&state.pool)
    .await?;
    let positions = queue_positions(state).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let steps = steps_of(state, &r.id).await?;
        let snapshots = snapshots::for_deploy(state, &r.id).await?;
        out.push(Deploy {
            queue_position: positions.get(&r.id).copied(),
            id: r.id,
            app_id: r.app_id,
            app_slug: r.app_slug,
            trigger: r.trigger,
            git_ref: r.git_ref,
            commit_sha: r.commit_sha,
            commit_message: r.commit_message,
            author: r.author,
            state: r.state,
            outcome: r.outcome,
            failure_reason: r.failure_reason,
            release_id: r.release_id,
            restore_deploy_id: r.restore_deploy_id,
            started_at: time::utc(r.started_at),
            finished_at: time::utc_opt(r.finished_at),
            duration_secs: r.duration_secs.map(|s| s.max(0) as u64),
            steps,
            snapshots,
        });
    }
    Ok(out)
}

async fn steps_of(state: &State, deploy_id: &str) -> anyhow::Result<Vec<Step>> {
    let rows = sqlx::query!(
        r#"SELECT state AS "state!: DeployState", status AS "status!: StepStatus", note,
                  CAST(strftime('%s', finished_at) - strftime('%s', started_at) AS INTEGER) AS "elapsed_secs: i64"
           FROM deploy_steps WHERE deploy_id = ? ORDER BY position"#,
        deploy_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Step {
            state: r.state,
            status: r.status,
            elapsed_secs: r.elapsed_secs.map(|s| s.max(0) as u64),
            note: r.note,
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self};

    pub fn commit(sha: &str) -> Commit {
        Commit {
            sha: Some(sha.into()),
            message: Some("Add reconciliation window".into()),
            author: Some("saeed".into()),
        }
    }

    #[tokio::test]
    async fn a_deploy_walks_its_steps_and_records_what_it_skipped() {
        let (_d, state) = state().await;
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let d = create(&state, &app, Trigger::Manual, "main", &commit("a3f9c2d4"))
            .await
            .unwrap();
        assert_eq!(d.state, Some(DeployState::Queued));
        assert_eq!(d.app_slug, "ledger");
        assert_eq!(d.queue_position, Some(0));
        enter(&state, &d.id, DeployState::Cloning).await.unwrap();
        skip(
            &state,
            &d.id,
            DeployState::InstallingSystemPackages,
            "all 0 packages already present",
        )
        .await
        .unwrap();
        enter(&state, &d.id, DeployState::Building).await.unwrap();
        let d = by_id(&state, &d.id).await.unwrap().unwrap();
        let statuses: Vec<_> = d.steps.iter().map(|s| (s.state, s.status)).collect();
        assert_eq!(statuses[0], (DeployState::Queued, StepStatus::Done));
        assert_eq!(statuses[1], (DeployState::Cloning, StepStatus::Done));
        assert_eq!(
            statuses[2],
            (DeployState::InstallingSystemPackages, StepStatus::Skipped)
        );
        assert_eq!(
            statuses[3],
            (DeployState::InstallingDeps, StepStatus::Pending)
        );
        assert_eq!(statuses[4], (DeployState::Building, StepStatus::Active));
        assert_eq!(
            d.steps[2].note.as_deref(),
            Some("all 0 packages already present")
        );
        assert_eq!(
            d.steps.len(),
            12,
            "every state is listed so the ladder can draw the pending ones"
        );
        assert!(d.steps[1].elapsed_secs.is_some());
        assert!(d.is_running());
        assert_eq!(d.queue_position, None);

        finish(&state, &d.id, Outcome::Failed, Some("boom"), None)
            .await
            .unwrap();
        let d = by_id(&state, &d.id).await.unwrap().unwrap();
        assert_eq!(d.outcome, Some(Outcome::Failed));
        assert!(d.state.is_none());
        assert_eq!(d.steps[4].status, StepStatus::Failed);
        assert!(d.finished_at.as_ref().unwrap().ends_with('Z'));
        assert!(d.duration_secs.is_some());
        assert_eq!(d.failure_reason.as_deref(), Some("boom"));
        assert_eq!(
            serde_json::to_value(&d).unwrap()["outcome"],
            "Failed",
            "the panel reads PascalCase outcomes"
        );
        assert_eq!(
            serde_json::to_value(&d).unwrap()["steps"][0]["status"],
            "done"
        );
    }

    #[tokio::test]
    async fn deploys_left_running_by_a_crash_are_failed_on_start() {
        let (_d, state) = state().await;
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let d = create(&state, &app, Trigger::Webhook, "main", &Commit::default())
            .await
            .unwrap();
        enter(&state, &d.id, DeployState::Building).await.unwrap();
        assert!(running(&state).await.unwrap().is_some());
        assert_eq!(abandon_unfinished(&state).await.unwrap(), 1);
        assert_eq!(abandon_unfinished(&state).await.unwrap(), 0);
        let d = by_id(&state, &d.id).await.unwrap().unwrap();
        assert_eq!(d.outcome, Some(Outcome::Failed));
        assert_eq!(d.failure_reason.as_deref(), Some(RESTARTED));
        assert!(running(&state).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn queue_positions_count_the_running_one_and_those_ahead() {
        let (_d, state) = state().await;
        let a = apps::create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let b = apps::create(&state, new_app("b", &[("/", "main", false)]))
            .await
            .unwrap();
        let first = create(&state, &a, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        enter(&state, &first.id, DeployState::Cloning)
            .await
            .unwrap();
        let second = create(&state, &b, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        let third = create(&state, &a, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        assert_eq!(
            by_id(&state, &second.id)
                .await
                .unwrap()
                .unwrap()
                .queue_position,
            Some(1)
        );
        assert_eq!(
            by_id(&state, &third.id)
                .await
                .unwrap()
                .unwrap()
                .queue_position,
            Some(2)
        );
        assert_eq!(
            queued_for(&state, &b.id).await.unwrap().unwrap().id,
            second.id
        );
        assert_eq!(list(&state, Some(&a.id), 10).await.unwrap().len(), 2);
        assert!(delete(&state, &third.id).await.unwrap());
        assert!(!delete(&state, &third.id).await.unwrap());
    }
}
