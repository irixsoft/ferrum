use super::handler::{Ferrum, ToolResult, finish};
use crate::routes::error::{ApiError, ApiResult};
use crate::routes::{apps, databases, deploys, host, logs, nginx};
use ferrum_core::certs;
use ferrum_core::deploy::{self, log};
use ferrum_core::logs::{DEFAULT_LINES, MAX_LINES};
use ferrum_core::metrics::HOST;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;

const HISTORY_DEFAULT: u32 = 20;
const HISTORY_MAX: u32 = 100;

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Slug {
    /// The application's slug, as shown in the panel's URL.
    pub slug: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct History {
    /// The application's slug.
    pub slug: String,
    /// How many deploys, newest first. Default 20, at most 100.
    pub limit: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeployId {
    /// The deploy's id, from deploy_history or the deploy tool.
    pub deploy_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AppLogs {
    /// The application's slug.
    pub slug: String,
    /// `app` for the process's journal, `access` or `error` for nginx. Default `app`.
    pub source: Option<String>,
    /// How many lines from the end. Default 200, at most 2000.
    pub lines: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Metrics {
    /// `host` for the whole server, or an application's slug.
    pub scope: String,
    /// `1h`, `24h` or `7d`. Default `24h`.
    pub range: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DatabaseName {
    /// The database's name.
    pub name: String,
}

#[tool_router(router = read_router, vis = "pub(super)")]
impl Ferrum {
    #[tool(
        name = "list_apps",
        description = "List every application with its runtime, repository, domains and status (new, building, live, stopped, failed or maintenance).",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_apps(&self) -> ToolResult {
        finish(apps::listed(&self.state).await.map_err(ApiError::from))
    }

    #[tool(
        name = "get_app",
        description = "One application in full: configuration, environment variable keys (never values), current release, certificates, linked databases, Redis, memory and CPU.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn get_app(&self, Parameters(args): Parameters<Slug>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                Ok::<_, ApiError>(apps::detail(&self.state, &found).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "deploy_history",
        description = "An application's deploys, newest first, each with its trigger, commit, outcome, steps and snapshots.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn deploy_history(&self, Parameters(args): Parameters<History>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                let limit = args.limit.unwrap_or(HISTORY_DEFAULT).clamp(1, HISTORY_MAX);
                Ok::<_, ApiError>(deploy::list(&self.state.db, Some(&found.id), limit).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "deploy_log",
        description = "Every stored line of one deploy's log so far, in order; call again while a deploy runs to see more.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn deploy_log(&self, Parameters(args): Parameters<DeployId>) -> ToolResult {
        finish(self.deploy_lines(&args.deploy_id, false).await)
    }

    #[tool(
        name = "build_log",
        description = "Only what the install, build and migrate commands printed during one deploy, without Ferrum's own step markers.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn build_log(&self, Parameters(args): Parameters<DeployId>) -> ToolResult {
        finish(self.deploy_lines(&args.deploy_id, true).await)
    }

    #[tool(
        name = "app_logs",
        description = "Tail an application's journald, nginx access or nginx error log; newest line last.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn app_logs(&self, Parameters(args): Parameters<AppLogs>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                let source = logs::source(args.source.as_deref())?;
                let lines = args.lines.unwrap_or(DEFAULT_LINES).clamp(1, MAX_LINES);
                Ok::<_, ApiError>(ferrum_core::logs::tail(
                    self.state.platform.as_ref(),
                    &found,
                    source,
                    lines,
                )?)
            }
            .await,
        )
    }

    #[tool(
        name = "metrics",
        description = "CPU and memory over time for the host (memory in percent) or one application (memory in MB), bucketed to at most 360 points.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn metrics(&self, Parameters(args): Parameters<Metrics>) -> ToolResult {
        finish(
            async {
                let since = host::window_secs(args.range.as_deref())?;
                if args.scope == HOST {
                    return Ok::<_, ApiError>(host::host_series(&self.state, since).await?);
                }
                let found = apps::find(&self.state, &args.scope).await?;
                Ok(host::app_series(&self.state, &found, since).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "nginx_config",
        description = "An application's nginx files: the managed server block Ferrum writes and the custom directives file the user may edit.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn nginx_config(&self, Parameters(args): Parameters<Slug>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                Ok::<_, ApiError>(nginx::files(&self.state, &found)?)
            }
            .await,
        )
    }

    #[tool(
        name = "certificate_status",
        description = "Each of an application's domains with its certificate state: issued with an expiry, pending, or failed with the reason and the next attempt.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn certificate_status(&self, Parameters(args): Parameters<Slug>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                Ok::<_, ApiError>(
                    certs::statuses(&self.state.db, self.state.platform.as_ref(), &found).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "list_databases",
        description = "Every PostgreSQL database with its role, connection limit, extensions, linked applications, size and active connections.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn list_databases(&self) -> ToolResult {
        finish(databases::listed(&self.state).await)
    }

    #[tool(
        name = "database_info",
        description = "One database with a connection URL template; the password is written as <password> and is only ever in the linked application's .env.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn database_info(&self, Parameters(args): Parameters<DatabaseName>) -> ToolResult {
        finish(databases::detail(&self.state, &args.name).await)
    }

    #[tool(
        name = "system_status",
        description = "The host: Ferrum version, uptime, load, memory, disk, the services with a sentence each, and under `security` the firewall, bans, updates and SSH state with any enable still running under `jobs`.",
        annotations(read_only_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn system_status(&self) -> ToolResult {
        finish(
            async {
                let build = crate::routes::version::build();
                let host =
                    ferrum_core::host::status(&self.state.db, self.state.platform.as_ref(), &build)
                        .await?;
                let mut value = serde_json::to_value(host).map_err(anyhow::Error::from)?;
                match crate::routes::security::view(&self.state).await {
                    Ok(security) => {
                        value["security"] =
                            serde_json::to_value(security).map_err(anyhow::Error::from)?;
                    }
                    Err(e) => {
                        value["security"] = serde_json::Value::Null;
                        value["security_error"] = serde_json::Value::String(e.message);
                    }
                }
                Ok::<_, ApiError>(value)
            }
            .await,
        )
    }
}

impl Ferrum {
    async fn deploy_lines(&self, id: &str, commands_only: bool) -> ApiResult<Vec<log::Line>> {
        let found = deploys::find_deploy(&self.state, id).await?;
        let lines = log::lines(&self.state.db, &found.id, 0).await?;
        Ok(lines
            .into_iter()
            .filter(|line| !commands_only || line.stream != "system")
            .collect())
    }
}
