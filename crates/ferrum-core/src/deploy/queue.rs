use super::log::Log;
use super::releases::Release;
use super::run::{Ctx, run};
use super::{Commit, Deploy, DeployState, Trigger, abandon_unfinished, by_id, create, queued_for};
use crate::apps::{self, App, Tracking};
use crate::github::webhook::Event;
use crate::state::State;
use tokio::sync::mpsc;

pub use crate::github::commits::{Head, head_of};

/// One worker, one build at a time. Routes enqueue and read.
#[derive(Clone)]
pub struct Deployer {
    tx: mpsc::UnboundedSender<String>,
    ctx: Ctx,
}

impl Deployer {
    pub fn start(ctx: Ctx) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let worker = ctx.clone();
        tokio::spawn(async move {
            match abandon_unfinished(&worker.state).await {
                Ok(0) => {}
                Ok(n) => {
                    tracing::warn!(count = n, "deploys interrupted by the restart were failed")
                }
                Err(e) => tracing::error!(error = ?e, "could not fail interrupted deploys"),
            }
            while let Some(id) = rx.recv().await {
                let waiting = by_id(&worker.state, &id)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|d| d.state == Some(DeployState::Queued));
                if !waiting {
                    continue;
                }
                match run(&worker, &id).await {
                    Ok(outcome) => tracing::info!(deploy = %id, ?outcome, "deploy finished"),
                    Err(e) => tracing::error!(deploy = %id, error = ?e, "deploy crashed"),
                }
            }
        });
        Self { tx, ctx }
    }

    pub fn ctx(&self) -> &Ctx {
        &self.ctx
    }

    pub fn log(&self) -> &Log {
        &self.ctx.log
    }

    /// A deploy already waiting for the app is retargeted rather than queued twice.
    pub async fn enqueue(
        &self,
        app: &App,
        trigger: Trigger,
        git_ref: &str,
        commit: &Commit,
    ) -> anyhow::Result<Deploy> {
        let state = &self.ctx.state;
        if let Some(waiting) = queued_for(state, &app.id).await? {
            super::retarget(state, &waiting.id, git_ref, commit).await?;
            return by_id(state, &waiting.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("the waiting deploy vanished"));
        }
        let deploy = create(state, app, trigger, git_ref, commit).await?;
        let _ = self.tx.send(deploy.id.clone());
        Ok(deploy)
    }

    pub async fn enqueue_rollback(
        &self,
        app: &App,
        release: &Release,
        restore_deploy_id: Option<&str>,
    ) -> anyhow::Result<Deploy> {
        let state = &self.ctx.state;
        let commit = Commit {
            sha: Some(release.commit_sha.clone()),
            message: release.commit_message.clone(),
            author: None,
        };
        let deploy = create(state, app, Trigger::Rollback, &release.git_ref, &commit).await?;
        super::set_rollback_target(state, &deploy.id, &release.id, restore_deploy_id).await?;
        let _ = self.tx.send(deploy.id.clone());
        by_id(state, &deploy.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the rollback vanished as it was queued"))
    }

    /// Every app tracking what the delivery describes gets a deploy.
    pub async fn react(&self, state: &State, event: &Event) -> anyhow::Result<Vec<Deploy>> {
        let mut queued = Vec::new();
        for app in apps::by_repository(state, event.repository()).await? {
            if !matches(&app, event) {
                continue;
            }
            let (git_ref, commit) = match event {
                Event::Push { commit_sha, .. } => (
                    app.git_ref.clone(),
                    Commit {
                        sha: Some(commit_sha.clone()),
                        ..Commit::default()
                    },
                ),
                Event::Release { tag, .. } => {
                    apps::set_git_ref(state, &app.id, tag).await?;
                    (tag.clone(), Commit::default())
                }
                _ => continue,
            };
            queued.push(
                self.enqueue(&app, Trigger::Webhook, &git_ref, &commit)
                    .await?,
            );
        }
        Ok(queued)
    }
}

pub fn matches(app: &App, event: &Event) -> bool {
    match event {
        Event::Push {
            repository,
            git_ref,
            ..
        } => {
            repository == &app.repository
                && app.tracking == Tracking::Branch
                && git_ref == &format!("refs/heads/{}", app.git_ref)
        }
        Event::Release { repository, .. } => {
            repository == &app.repository && app.tracking == Tracking::Releases
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{app, new_app, state};
    use crate::deploy::Outcome;
    use crate::deploy::tests::commit;
    use crate::github::Api;
    use crate::runtime::toolchain::Store;
    use ferrum_platform::FakePlatform;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn a_push_matches_the_tracked_branch_and_a_release_matches_release_tracking() {
        let branch = app("ledger");
        let push = |repository: &str, git_ref: &str| Event::Push {
            repository: repository.into(),
            git_ref: git_ref.into(),
            commit_sha: "a".into(),
        };
        let release = |repository: &str, tag: &str| Event::Release {
            repository: repository.into(),
            tag: tag.into(),
        };
        assert!(matches(
            &branch,
            &push("irixsoft/ledger", "refs/heads/main")
        ));
        assert!(!matches(
            &branch,
            &push("irixsoft/ledger", "refs/heads/dev")
        ));
        assert!(!matches(
            &branch,
            &push("irixsoft/ledger", "refs/tags/main")
        ));
        assert!(!matches(&branch, &release("irixsoft/ledger", "v1")));
        assert!(!matches(&branch, &push("someone/else", "refs/heads/main")));
        let mut releases = app("ledger");
        releases.tracking = Tracking::Releases;
        releases.git_ref = "v0.9".into();
        assert!(matches(&releases, &release("irixsoft/ledger", "v1.0")));
        assert!(!matches(
            &releases,
            &push("irixsoft/ledger", "refs/heads/main")
        ));
        assert!(!matches(&branch, &Event::Ping));
    }

    async fn ctx(state: &State, platform: &Arc<FakePlatform>) -> Ctx {
        let mut ctx = Ctx::new(
            state.clone(),
            platform.clone(),
            Api::at("http://127.0.0.1:1").with_fixed_token("ghs_fixed"),
            crate::http::client(),
            Store::at("/var/lib/ferrum/runtimes"),
        );
        ctx.health_interval = Duration::from_millis(20);
        ctx
    }

    async fn provisioned(state: &State, platform: &FakePlatform, slug: &str) -> App {
        let mut new = new_app(slug, &[("/", "main", false)]);
        new.runtime = crate::runtime::RuntimeKind::Static;
        new.output_dir = Some("dist".into());
        let app = apps::create(state, new).await.unwrap();
        crate::apps::provision::provision(state, platform, &app)
            .await
            .unwrap();
        app
    }

    async fn wait_for(state: &State, id: &str) -> Deploy {
        for _ in 0..500 {
            let d = by_id(state, id).await.unwrap().unwrap();
            if d.outcome.is_some() {
                return d;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("deploy {id} never finished");
    }

    #[tokio::test]
    async fn deploys_run_one_at_a_time_and_the_second_knows_its_place() {
        let (_d, state) = state().await;
        let platform = Arc::new(FakePlatform::new());
        let a = provisioned(&state, &platform, "a").await;
        let b = provisioned(&state, &platform, "b").await;
        let gate = platform.gate("bun run build");
        let deployer = Deployer::start(ctx(&state, &platform).await);

        let first = deployer
            .enqueue(&a, Trigger::Manual, "main", &commit("a1"))
            .await
            .unwrap();
        let second = deployer
            .enqueue(&b, Trigger::Manual, "main", &commit("b1"))
            .await
            .unwrap();
        for _ in 0..500 {
            if by_id(&state, &first.id).await.unwrap().unwrap().state == Some(DeployState::Building)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let waiting = by_id(&state, &second.id).await.unwrap().unwrap();
        assert_eq!(waiting.state, Some(DeployState::Queued));
        assert_eq!(waiting.queue_position, Some(1));

        gate.open();
        assert_eq!(
            wait_for(&state, &first.id).await.outcome,
            Some(Outcome::Live)
        );
        assert_eq!(
            wait_for(&state, &second.id).await.outcome,
            Some(Outcome::Live)
        );
        let calls = platform.calls();
        let a_swap = calls
            .iter()
            .position(|c| c.starts_with("symlink_swap /var/lib/ferrum/apps/a/releases"))
            .unwrap();
        let b_clone = calls
            .iter()
            .position(|c| c.starts_with("git_clone") && c.contains("/apps/b/"))
            .unwrap();
        assert!(
            a_swap < b_clone,
            "b must not start before a is live: {calls:#?}"
        );
    }

    #[tokio::test]
    async fn a_second_push_while_one_waits_replaces_the_waiting_sha_rather_than_queueing_twice() {
        let (_d, state) = state().await;
        let platform = Arc::new(FakePlatform::new());
        let a = provisioned(&state, &platform, "a").await;
        let b = provisioned(&state, &platform, "b").await;
        let gate = platform.gate("bun run build");
        let deployer = Deployer::start(ctx(&state, &platform).await);
        let blocking = deployer
            .enqueue(&a, Trigger::Manual, "main", &commit("a1"))
            .await
            .unwrap();
        let first = deployer
            .enqueue(&b, Trigger::Webhook, "main", &commit("b1"))
            .await
            .unwrap();
        let again = deployer
            .enqueue(&b, Trigger::Webhook, "main", &commit("b2"))
            .await
            .unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(again.commit_sha.as_deref(), Some("b2"));
        let queued: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM deploys WHERE app_id = ? AND state = 'Queued'",
        )
        .bind(&b.id)
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(queued, 1);
        gate.open();
        wait_for(&state, &blocking.id).await;
        assert_eq!(
            wait_for(&state, &again.id).await.outcome,
            Some(Outcome::Live)
        );
        assert!(
            platform
                .calls()
                .iter()
                .any(|c| c.contains("git_checkout") && c.ends_with(" b2"))
        );
    }

    #[tokio::test]
    async fn a_release_delivery_moves_the_tracked_ref_to_the_tag() {
        let (_d, state) = state().await;
        let platform = Arc::new(FakePlatform::new());
        let mut new = new_app("ledger", &[("/", "main", false)]);
        new.tracking = Tracking::Releases;
        new.git_ref = "v0.9".into();
        let app = apps::create(&state, new).await.unwrap();
        let deployer = Deployer::start(ctx(&state, &platform).await);
        let queued = deployer
            .react(
                &state,
                &Event::Release {
                    repository: "irixsoft/ledger".into(),
                    tag: "v1.0".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].git_ref, "v1.0");
        assert_eq!(queued[0].trigger, Trigger::Webhook);
        assert_eq!(
            apps::by_id(&state, &app.id).await.unwrap().unwrap().git_ref,
            "v1.0"
        );
        let none = deployer
            .react(
                &state,
                &Event::Push {
                    repository: "irixsoft/ledger".into(),
                    git_ref: "refs/heads/main".into(),
                    commit_sha: "a".into(),
                },
            )
            .await
            .unwrap();
        assert!(none.is_empty());
    }
}
