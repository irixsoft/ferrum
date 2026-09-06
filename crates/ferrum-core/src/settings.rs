use crate::state::State;
use ferrum_platform::Platform;
use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

pub const MEMORY_RESERVE_MB: u64 = 512;
pub const MEMORY_FLOOR_MB: u64 = 512;
pub const DEFAULT_BUILD_SECS: u64 = 20 * 60;
pub const DEFAULT_MIGRATE_SECS: u64 = 10 * 60;
pub const SECS_RANGE: RangeInclusive<u64> = 60..=7200;
const MEMORY_KEY: &str = "builds.memory_mb";
const BUILD_SECS_KEY: &str = "builds.build_secs";
const MIGRATE_SECS_KEY: &str = "builds.migrate_secs";

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildLimits {
    pub memory_mb: u64,
    pub build_secs: u64,
    pub migrate_secs: u64,
}

/// Everything but half a gigabyte, so the running apps and PostgreSQL keep theirs.
pub fn default_memory_mb(total_kb: u64) -> u64 {
    (total_kb / 1024)
        .saturating_sub(MEMORY_RESERVE_MB)
        .max(MEMORY_FLOOR_MB)
}

pub fn memory_ceiling_mb(total_kb: u64) -> u64 {
    (total_kb / 1024).max(MEMORY_FLOOR_MB)
}

async fn number(state: &State, key: &str) -> anyhow::Result<Option<u64>> {
    Ok(state
        .get_setting(key)
        .await?
        .and_then(|value| value.parse().ok()))
}

pub async fn build_limits(state: &State, platform: &dyn Platform) -> anyhow::Result<BuildLimits> {
    let total_kb = platform.total_memory_kb().unwrap_or(0);
    Ok(BuildLimits {
        memory_mb: number(state, MEMORY_KEY)
            .await?
            .unwrap_or_else(|| default_memory_mb(total_kb)),
        build_secs: number(state, BUILD_SECS_KEY)
            .await?
            .unwrap_or(DEFAULT_BUILD_SECS),
        migrate_secs: number(state, MIGRATE_SECS_KEY)
            .await?
            .unwrap_or(DEFAULT_MIGRATE_SECS),
    })
}

pub async fn set_build_limits(
    state: &State,
    platform: &dyn Platform,
    limits: BuildLimits,
) -> anyhow::Result<()> {
    let ceiling = memory_ceiling_mb(platform.total_memory_kb().unwrap_or(0));
    if !(MEMORY_FLOOR_MB..=ceiling).contains(&limits.memory_mb) {
        return Err(SettingsError::Invalid(format!(
            "The build memory limit must be between {MEMORY_FLOOR_MB} and {ceiling} MB on this host."
        ))
        .into());
    }
    for (what, secs) in [
        ("build", limits.build_secs),
        ("migration", limits.migrate_secs),
    ] {
        if !SECS_RANGE.contains(&secs) {
            return Err(SettingsError::Invalid(format!(
                "The {what} timeout must be between {} and {} seconds.",
                SECS_RANGE.start(),
                SECS_RANGE.end()
            ))
            .into());
        }
    }
    state
        .set_setting(MEMORY_KEY, &limits.memory_mb.to_string())
        .await?;
    state
        .set_setting(BUILD_SECS_KEY, &limits.build_secs.to_string())
        .await?;
    state
        .set_setting(MIGRATE_SECS_KEY, &limits.migrate_secs.to_string())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;
    use ferrum_platform::FakePlatform;

    #[test]
    fn the_default_limit_leaves_half_a_gigabyte_and_never_drops_below_it() {
        assert_eq!(default_memory_mb(2 * 1024 * 1024), 1536);
        assert_eq!(default_memory_mb(512 * 1024), 512);
        assert_eq!(default_memory_mb(0), 512);
        assert_eq!(memory_ceiling_mb(2 * 1024 * 1024), 2048);
        assert_eq!(memory_ceiling_mb(0), 512);
    }

    #[tokio::test]
    async fn limits_default_from_the_host_and_are_checked_against_it() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let defaults = build_limits(&state, &p).await.unwrap();
        assert_eq!(
            defaults,
            BuildLimits {
                memory_mb: 1536,
                build_secs: 1200,
                migrate_secs: 600
            }
        );
        let wanted = BuildLimits {
            memory_mb: 1200,
            build_secs: 900,
            migrate_secs: 120,
        };
        set_build_limits(&state, &p, wanted).await.unwrap();
        assert_eq!(build_limits(&state, &p).await.unwrap(), wanted);
        for bad in [
            BuildLimits {
                memory_mb: 511,
                ..wanted
            },
            BuildLimits {
                memory_mb: 2049,
                ..wanted
            },
            BuildLimits {
                build_secs: 59,
                ..wanted
            },
            BuildLimits {
                migrate_secs: 7201,
                ..wanted
            },
        ] {
            let e = set_build_limits(&state, &p, bad).await.unwrap_err();
            assert!(e.downcast_ref::<SettingsError>().is_some(), "{e}");
        }
        assert_eq!(build_limits(&state, &p).await.unwrap(), wanted);
        assert!(
            set_build_limits(
                &state,
                &p,
                BuildLimits {
                    memory_mb: 2048,
                    ..wanted
                }
            )
            .await
            .is_ok(),
            "the whole of RAM is allowed, on the user's head"
        );
    }
}
