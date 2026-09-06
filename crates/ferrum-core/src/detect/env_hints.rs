use super::RepoTree;
use serde::{Deserialize, Serialize};

pub const DOTENV_FILES: [&str; 4] = [
    ".env.example",
    ".env.sample",
    ".env.template",
    ".env.local.example",
];
pub const SCHEMA_FILES: [&str; 5] = [
    "src/env.ts",
    "src/lib/env.ts",
    "env.ts",
    "env.mjs",
    "src/config/env.ts",
];
pub const ECOSYSTEM_GLOB: &str = "ecosystem.config.*";

const FRAMEWORK_SET: [&str; 4] = ["NODE_ENV", "NEXT_PHASE", "NEXT_RUNTIME", "CI"];
const FRAMEWORK_PREFIXES: [&str; 1] = ["VERCEL_"];
const MANAGED: [&str; 4] = ["PORT", "HOST", "DATABASE_URL", "REDIS_URL"];
const APP_URL_KEYS: [&str; 4] = [
    "NEXT_PUBLIC_APP_URL",
    "NEXT_PUBLIC_SITE_URL",
    "APP_URL",
    "SITE_URL",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvHint {
    pub key: String,
    pub source: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub suggest_app_url: bool,
}

pub fn valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn dotenv_keys(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if valid_key(key) && !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
        }
    }
    keys
}

/// `KEY: z.string()` and `KEY: process.env.KEY` lines; a chain ending in `.optional()` or
/// carrying `.default(` on the same line marks the key optional.
pub fn schema_keys(text: &str) -> Vec<(String, bool)> {
    let mut keys: Vec<(String, bool)> = Vec::new();
    for line in text.lines().map(str::trim) {
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches(|c| c == '"' || c == '\'');
        if !is_upper_key(key) {
            continue;
        }
        let rest = rest.trim();
        if !(rest.contains("z.") || rest.contains("process.env") || rest.contains("v.")) {
            continue;
        }
        let optional = rest.contains(".optional()") || rest.contains(".default(");
        match keys.iter_mut().find(|(k, _)| k == key) {
            Some((_, o)) => *o = *o || optional,
            None => keys.push((key.to_string(), optional)),
        }
    }
    keys
}

/// The keys of the first `env: {` block in a PM2 ecosystem file.
pub fn ecosystem_keys(text: &str) -> Vec<String> {
    let Some(start) = text.find("env:") else {
        return Vec::new();
    };
    let Some(open) = text[start..].find('{') else {
        return Vec::new();
    };
    let body = &text[start + open + 1..];
    let mut depth = 0usize;
    let mut end = body.len();
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' if depth == 0 => {
                end = i;
                break;
            }
            '}' => depth -= 1,
            _ => {}
        }
    }
    body[..end]
        .lines()
        .filter_map(|line| line.trim().split_once(':'))
        .map(|(key, _)| {
            key.trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string()
        })
        .filter(|key| is_upper_key(key))
        .collect()
}

fn is_upper_key(key: &str) -> bool {
    valid_key(key) && key.chars().any(|c| c.is_ascii_uppercase()) && key.to_ascii_uppercase() == key
}

pub fn is_framework_set(key: &str) -> bool {
    FRAMEWORK_SET.contains(&key) || FRAMEWORK_PREFIXES.iter().any(|p| key.starts_with(p))
}

pub fn is_managed(key: &str) -> bool {
    MANAGED.contains(&key) || key.ends_with("_DATABASE_URL")
}

pub fn hints(tree: &RepoTree) -> Vec<EnvHint> {
    let mut found: Vec<EnvHint> = Vec::new();
    let mut add = |key: &str, source: &str, optional: bool| {
        if is_framework_set(key) || is_managed(key) {
            return;
        }
        match found.iter_mut().find(|h| h.key == key) {
            Some(existing) => existing.optional = existing.optional || optional,
            None => found.push(EnvHint {
                key: key.to_string(),
                source: format!("from {source}"),
                optional,
                suggest_app_url: APP_URL_KEYS.contains(&key),
            }),
        }
    };
    for file in DOTENV_FILES {
        if let Some(text) = tree.read(file) {
            for key in dotenv_keys(text) {
                add(&key, file, false);
            }
        }
    }
    for file in SCHEMA_FILES {
        if let Some(text) = tree.read(file) {
            for (key, optional) in schema_keys(text) {
                add(&key, file, optional);
            }
        }
    }
    for file in tree.matching(ECOSYSTEM_GLOB) {
        if let Some(text) = tree.read(file) {
            for key in ecosystem_keys(text) {
                add(&key, file, false);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotenv_example_yields_its_keys_in_order_without_comments_or_values() {
        let keys = dotenv_keys(
            "# database\nDATABASE_URL=postgres://x\nexport SMTP_HOST=smtp.example.com\n\nSMTP_PORT = 587\nbad key=1\nSMTP_HOST=again\n",
        );
        assert_eq!(keys, vec!["DATABASE_URL", "SMTP_HOST", "SMTP_PORT"]);
    }

    #[test]
    fn a_zod_schema_names_its_keys_and_which_are_optional() {
        let schema = r#"
import { z } from "zod";
export const env = createEnv({
  server: {
    DATABASE_URL: z.string().url(),
    SMTP_HOST: z.string().optional(),
    LOG_LEVEL: z.enum(["info", "debug"]).default("info"),
  },
  client: {
    NEXT_PUBLIC_APP_URL: z.string().url(),
  },
  runtimeEnv: {
    DATABASE_URL: process.env.DATABASE_URL,
    NEXT_PUBLIC_APP_URL: process.env.NEXT_PUBLIC_APP_URL,
  },
});
"#;
        assert_eq!(
            schema_keys(schema),
            vec![
                ("DATABASE_URL".to_string(), false),
                ("SMTP_HOST".to_string(), true),
                ("LOG_LEVEL".to_string(), true),
                ("NEXT_PUBLIC_APP_URL".to_string(), false),
            ]
        );
    }

    #[test]
    fn an_ecosystem_file_yields_the_env_block_only() {
        let config = r#"
module.exports = {
  apps: [{
    name: "web",
    script: "bun run start",
    env: {
      NODE_ENV: "production",
      PORT: 3000,
      SENTRY_DSN: "https://x",
    },
    env_production: { EXTRA: "1" },
  }],
};
"#;
        assert_eq!(
            ecosystem_keys(config),
            vec!["NODE_ENV", "PORT", "SENTRY_DSN"]
        );
        assert!(ecosystem_keys("module.exports = {}").is_empty());
    }

    #[test]
    fn hints_union_the_sources_and_drop_what_ferrum_or_the_framework_sets() {
        let tree = RepoTree::from_files(&[
            (
                ".env.example",
                "DATABASE_URL=\nNEXT_PUBLIC_APP_URL=http://localhost:3000\nSMTP_HOST=\nNODE_ENV=development\nPORT=3000\nVERCEL_URL=\nREDIS_URL=\nANALYTICS_DATABASE_URL=\n",
            ),
            (
                "src/env.ts",
                "server: {\n  SMTP_HOST: z.string().optional(),\n  STRIPE_KEY: z.string(),\n},\n",
            ),
            (
                "ecosystem.config.cjs",
                "env: {\n  NODE_ENV: 'production',\n  SENTRY_DSN: '',\n}\n",
            ),
        ]);
        let found = hints(&tree);
        let keys: Vec<&str> = found.iter().map(|h| h.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "NEXT_PUBLIC_APP_URL",
                "SMTP_HOST",
                "STRIPE_KEY",
                "SENTRY_DSN"
            ]
        );
        let smtp = found.iter().find(|h| h.key == "SMTP_HOST").unwrap();
        assert_eq!(smtp.source, "from .env.example");
        assert!(smtp.optional, "the schema says optional");
        let url = found
            .iter()
            .find(|h| h.key == "NEXT_PUBLIC_APP_URL")
            .unwrap();
        assert!(url.suggest_app_url);
        assert!(!smtp.suggest_app_url);
        let stripe = found.iter().find(|h| h.key == "STRIPE_KEY").unwrap();
        assert_eq!(stripe.source, "from src/env.ts");
        assert_eq!(
            found.iter().find(|h| h.key == "SENTRY_DSN").unwrap().source,
            "from ecosystem.config.cjs"
        );
    }
}
