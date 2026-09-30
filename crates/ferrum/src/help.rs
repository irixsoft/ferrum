use rust_embed::RustEmbed;
use serde::Serialize;

#[derive(RustEmbed)]
#[folder = "../../help/"]
struct Files;

const ORDER: [&str; 10] = [
    "ferrum-toml",
    "processes",
    "domains",
    "env",
    "aptfile",
    "procfile",
    "databases",
    "wildcards",
    "notifications",
    "connect-from-your-machine",
];

#[derive(Debug, Clone, Serialize)]
pub struct Topic {
    pub slug: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

fn read(slug: &str) -> Option<String> {
    let file = Files::get(&format!("{slug}.md"))?;
    String::from_utf8(file.data.into_owned()).ok()
}

/// The first `# ` line names the topic.
fn title_of(body: &str) -> String {
    body.lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or("Help")
        .trim()
        .to_string()
}

pub fn list() -> Vec<Topic> {
    ORDER
        .iter()
        .filter_map(|slug| {
            let body = read(slug)?;
            Some(Topic {
                slug: slug.to_string(),
                title: title_of(&body),
                body: None,
            })
        })
        .collect()
}

pub fn get(slug: &str) -> Option<Topic> {
    if !ORDER.contains(&slug) {
        return None;
    }
    let body = read(slug)?;
    Some(Topic {
        slug: slug.to_string(),
        title: title_of(&body),
        body: Some(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_topic_is_embedded_with_a_title_and_an_example_where_it_claims_one() {
        let topics = list();
        assert_eq!(topics.len(), ORDER.len(), "{topics:#?}");
        for topic in &topics {
            assert!(
                !topic.title.is_empty() && topic.title != "Help",
                "{topic:?}"
            );
            let full = get(&topic.slug).unwrap();
            assert!(full.body.as_deref().unwrap().starts_with("# "));
        }
        assert_eq!(list()[0].slug, "ferrum-toml");
        assert!(get("nope").is_none());
        assert!(get("../Cargo").is_none());
        let toml = get("ferrum-toml").unwrap();
        assert!(toml.body.unwrap().contains("[processes.web]"));
    }
}
