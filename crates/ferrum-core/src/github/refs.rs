use super::Api;
use crate::state::State;
use anyhow::Context;
use serde::{Deserialize, Serialize};

const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub sha: String,
}

#[derive(Deserialize)]
struct TagEntry {
    name: String,
    commit: CommitRef,
}

#[derive(Deserialize)]
struct CommitRef {
    sha: String,
}

impl Api {
    /// The repository's tags, newest first as far as their names say.
    pub async fn tags(&self, state: &State, repository: &str) -> anyhow::Result<Vec<Tag>> {
        let client = self.installed(state, super::owner_of(repository)).await?;
        let mut found: Vec<Tag> = Vec::new();
        for page in 1..=MAX_PAGES {
            let route = format!("/repos/{repository}/tags?per_page={PER_PAGE}&page={page}");
            let batch: Vec<TagEntry> = client
                .get(&route, None::<&()>)
                .await
                .with_context(|| format!("listing the tags of {repository}"))?;
            let full = batch.len() == PER_PAGE;
            found.extend(batch.into_iter().map(|t| Tag {
                name: t.name,
                sha: t.commit.sha,
            }));
            if !full {
                break;
            }
        }
        sort_newest_first(&mut found);
        Ok(found)
    }
}

/// Version-shaped names sort by their numbers, descending; the rest keep GitHub's order after them.
pub fn sort_newest_first(tags: &mut [Tag]) {
    tags.sort_by_cached_key(|t| match version_key(&t.name) {
        Some(parts) => (
            0,
            parts.into_iter().map(|p| u64::MAX - p).collect::<Vec<_>>(),
        ),
        None => (1, Vec::new()),
    });
}

fn version_key(name: &str) -> Option<Vec<u64>> {
    let digits = name.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '-');
    let core = digits.split(['-', '+']).next().unwrap_or(digits);
    let parts: Vec<u64> = core
        .split('.')
        .map(|p| p.parse().ok())
        .collect::<Option<Vec<_>>>()?;
    (!parts.is_empty()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(name: &str) -> Tag {
        Tag {
            name: name.into(),
            sha: "0".repeat(40),
        }
    }

    #[test]
    fn versions_sort_by_number_not_by_text_and_odd_names_come_last() {
        let mut tags = vec![
            tag("v1.9.0"),
            tag("release-candidate"),
            tag("v1.10.0"),
            tag("v1.10.0-rc.1"),
            tag("2.0"),
            tag("nightly"),
        ];
        sort_newest_first(&mut tags);
        let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "2.0",
                "v1.10.0",
                "v1.10.0-rc.1",
                "v1.9.0",
                "release-candidate",
                "nightly"
            ]
        );
    }
}
