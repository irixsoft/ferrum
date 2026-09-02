use super::handler::{Ferrum, ToolResult, finish, refusal};
use crate::routes::error::ApiError;
use crate::routes::{apps, databases, deploys, nginx};
use ferrum_core::apps::AppChanges;
use ferrum_core::apps::env::EnvChange;
use ferrum_core::deploy::Trigger;
use ferrum_core::postgres::NewDatabase;
use ferrum_core::settings::{self, SettingsError};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{schemars, tool, tool_router};
use serde::Deserialize;

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Var {
    /// The variable's name.
    pub key: String,
    /// The value. Leave it out to keep what is stored for that key.
    pub value: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SetEnv {
    /// The application's slug.
    pub slug: String,
    /// The complete set of variables; any stored key not listed here is removed.
    pub vars: Vec<Var>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Custom {
    /// The application's slug.
    pub slug: String,
    /// The whole custom directives file, included inside the app's server block.
    pub custom: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Slug {
    /// The application's slug.
    pub slug: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DeployRef {
    /// The application's slug.
    pub slug: String,
    /// A branch, tag or commit. Leave it out to deploy what the application tracks.
    pub git_ref: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Rollback {
    /// The application's slug.
    pub slug: String,
    /// The release to go back to, from get_app's current_release or deploy_history's release_id.
    pub release_id: String,
    /// Also restore the database snapshot that deploy took before its migration. Off unless given.
    pub restore_deploy_id: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct CreateDatabase {
    /// The database's name; lowercase letters, digits and underscores.
    pub name: String,
    /// The role's connection limit. Default 20.
    pub connection_limit: Option<u32>,
    /// Extensions to enable, from the list system_status names.
    pub extensions: Option<Vec<String>>,
    /// Link the new database to this application and write DATABASE_URL into its .env.
    pub app_slug: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AddDomain {
    /// The application's slug.
    pub slug: String,
    /// The domain to add; its DNS must already point at this server for the certificate.
    pub domain: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct Limits {
    /// An application's slug to change its own limits; leave it out to change the build limits.
    pub slug: Option<String>,
    /// The application's memory limit in MB.
    pub memory_mb: Option<u32>,
    /// The application's CPU quota in percent of one core.
    pub cpu_percent: Option<u32>,
    /// Memory for a build in MB, without a slug.
    pub build_memory_mb: Option<u64>,
    /// Seconds a build may take, without a slug.
    pub build_secs: Option<u64>,
    /// Seconds a migration may take, without a slug.
    pub migrate_secs: Option<u64>,
}

const NOTHING_TO_CHANGE: &str = "Name at least one limit to change.";
const APP_LIMITS_ONLY: &str = "With a slug, only memory_mb and cpu_percent apply.";
const BUILD_LIMITS_ONLY: &str =
    "Without a slug, only build_memory_mb, build_secs and migrate_secs apply.";

#[tool_router(router = write_router, vis = "pub(super)")]
impl Ferrum {
    #[tool(
        name = "set_env",
        description = "Replace an application's environment variables and rewrite its .env; the running process keeps its old values until the next deploy or restart.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_env(&self, Parameters(args): Parameters<SetEnv>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                let vars: Vec<EnvChange> = args
                    .vars
                    .into_iter()
                    .map(|v| EnvChange {
                        key: v.key,
                        value: v.value,
                    })
                    .collect();
                apps::replace_env(&self.state, &found, &vars).await?;
                Ok::<_, ApiError>(serde_json::json!({ "slug": found.slug, "keys": vars.len() }))
            }
            .await,
        )
    }

    #[tool(
        name = "edit_nginx_directives",
        description = "Replace an application's custom nginx directives file and reload nginx; if nginx rejects it the previous file is restored and nginx's error is returned.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn edit_nginx_directives(&self, Parameters(args): Parameters<Custom>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                nginx::set_custom_file(&self.state, &found, &args.custom)?;
                Ok::<_, ApiError>(nginx::files(&self.state, &found)?)
            }
            .await,
        )
    }

    #[tool(
        name = "restart_app",
        description = "Restart an application's systemd unit now; refused for a static site, during a deploy, or before the first deploy.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn restart_app(&self, Parameters(args): Parameters<Slug>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                apps::restart_unit(&self.state, &found).await?;
                Ok::<_, ApiError>(serde_json::json!({ "slug": found.slug, "restarted": true }))
            }
            .await,
        )
    }

    #[tool(
        name = "deploy",
        description = "Queue a deploy of a branch, tag or commit and return it with its queue position; follow it with deploy_log until its outcome is set.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn deploy(&self, Parameters(args): Parameters<DeployRef>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                deploys::queue(
                    &self.state,
                    &found,
                    args.git_ref.as_deref(),
                    Trigger::Manual,
                )
                .await
            }
            .await,
        )
    }

    #[tool(
        name = "rollback",
        description = "Queue a rollback that repoints the application at an earlier release without a build; a database snapshot is restored only when restore_deploy_id names the deploy that took it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn rollback(&self, Parameters(args): Parameters<Rollback>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                deploys::queue_rollback(
                    &self.state,
                    &found,
                    &args.release_id,
                    args.restore_deploy_id.as_deref(),
                )
                .await
            }
            .await,
        )
    }

    #[tool(
        name = "create_database",
        description = "Create a PostgreSQL database with its own role and password, and link it to an application when app_slug is given.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_database(&self, Parameters(args): Parameters<CreateDatabase>) -> ToolResult {
        let new = NewDatabase {
            name: args.name,
            connection_limit: args.connection_limit,
            extensions: args.extensions.unwrap_or_default(),
        };
        finish(databases::create_database(&self.state, new, args.app_slug.as_deref()).await)
    }

    #[tool(
        name = "add_domain",
        description = "Add a domain to an application, rewrite its nginx server block, and request a certificate in the background once DNS points here.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn add_domain(&self, Parameters(args): Parameters<AddDomain>) -> ToolResult {
        finish(
            async {
                let found = apps::find(&self.state, &args.slug).await?;
                let domain = args.domain.trim().to_ascii_lowercase();
                let mut domains = found.domains.clone();
                if !domains.contains(&domain) {
                    domains.push(domain);
                }
                let changes = AppChanges {
                    domains: Some(domains),
                    ..AppChanges::default()
                };
                apps::apply(&self.state, &found.slug, changes).await
            }
            .await,
        )
    }

    #[tool(
        name = "adjust_resource_limits",
        description = "With a slug, set that application's memory_mb and cpu_percent and rewrite its unit; without one, set the build memory and the build and migrate timeouts for the next deploy.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn adjust_resource_limits(&self, Parameters(args): Parameters<Limits>) -> ToolResult {
        let app_limits = args.memory_mb.is_some() || args.cpu_percent.is_some();
        let build_limits = args.build_memory_mb.is_some()
            || args.build_secs.is_some()
            || args.migrate_secs.is_some();
        if !app_limits && !build_limits {
            return Ok(refusal(NOTHING_TO_CHANGE));
        }
        match args.slug {
            Some(slug) => {
                if build_limits {
                    return Ok(refusal(APP_LIMITS_ONLY));
                }
                let changes = AppChanges {
                    memory_mb: args.memory_mb,
                    cpu_percent: args.cpu_percent,
                    ..AppChanges::default()
                };
                finish(apps::apply(&self.state, &slug, changes).await)
            }
            None => {
                if app_limits {
                    return Ok(refusal(BUILD_LIMITS_ONLY));
                }
                finish(
                    async {
                        let platform = self.state.platform.as_ref();
                        let mut limits = settings::build_limits(&self.state.db, platform).await?;
                        limits.memory_mb = args.build_memory_mb.unwrap_or(limits.memory_mb);
                        limits.build_secs = args.build_secs.unwrap_or(limits.build_secs);
                        limits.migrate_secs = args.migrate_secs.unwrap_or(limits.migrate_secs);
                        settings::set_build_limits(&self.state.db, platform, limits)
                            .await
                            .map_err(|e| match e.downcast_ref::<SettingsError>() {
                                Some(_) => ApiError::bad_request(e.to_string()),
                                None => e.into(),
                            })?;
                        Ok::<_, ApiError>(settings::build_limits(&self.state.db, platform).await?)
                    }
                    .await,
                )
            }
        }
    }
}
