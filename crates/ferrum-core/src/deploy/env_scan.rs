const FRAMEWORK_SET: [&str; 4] = ["NODE_ENV", "NEXT_PHASE", "NEXT_RUNTIME", "CI"];
const FRAMEWORK_PREFIXES: [&str; 1] = ["VERCEL_"];
const UNSET_PHRASES: [&str; 8] = [
    " is not set",
    " is not defined",
    " is not configured",
    " is required",
    " is missing",
    " must be set",
    " must be defined",
    " was not found",
];
const MISSING_WORD: &str = "missing";
const INVALID_BLOCK: &str = "Invalid environment variables";

fn is_framework_set(key: &str) -> bool {
    FRAMEWORK_SET.contains(&key) || FRAMEWORK_PREFIXES.iter().any(|p| key.starts_with(p))
}

fn valid_key(key: &str) -> bool {
    crate::apps::env::valid_key(key).is_ok()
}

fn identifier(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    &text[..end]
}

/// The variables a failed command complained about, read from validator output shapes.
pub fn keys_in_failure(lines: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    let mut add = |key: &str| {
        if is_upper_key(key) && !keys.iter().any(|k| k == key) {
            keys.push(key.to_string());
        }
    };
    let mut in_block = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.contains(INVALID_BLOCK) {
            in_block = true;
            for key in keys_before_brackets(trimmed) {
                add(key);
            }
            continue;
        }
        if in_block {
            let found = keys_before_brackets(trimmed);
            if found.is_empty() && !trimmed.starts_with('{') && !trimmed.starts_with('}') {
                in_block = false;
            }
            for key in found {
                add(key);
            }
        }
        for key in path_keys(trimmed) {
            add(key);
        }
        let lower = trimmed.to_ascii_lowercase();
        for phrase in UNSET_PHRASES {
            for (at, _) in lower.match_indices(phrase) {
                add(last_word(&trimmed[..at]));
            }
        }
        if lower.contains(MISSING_WORD) {
            for word in words(trimmed) {
                add(word);
            }
        }
    }
    keys
}

fn last_word(text: &str) -> &str {
    let text = text.trim_end();
    let start = text
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map(|i| i + 1)
        .unwrap_or(0);
    &text[start..]
}

/// `"path": ["KEY"]` from a serialised zod issue, `path: [ 'KEY' ]` from a logged one.
fn path_keys(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices("path") {
        let rest = line[at + 4..].trim_start_matches('"').trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('[') else {
            continue;
        };
        let rest = rest.trim_start().trim_start_matches(['"', '\'']);
        let key = identifier(rest);
        if !key.is_empty() {
            found.push(key);
        }
    }
    found
}

/// `KEY: [ 'Required' ]` entries of a t3-env style report.
fn keys_before_brackets(line: &str) -> Vec<&str> {
    let mut found = Vec::new();
    for (at, _) in line.match_indices(':') {
        let before = line[..at].trim_end();
        let start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map(|i| i + 1)
            .unwrap_or(0);
        let key = &before[start..];
        let after = line[at + 1..].trim_start();
        if is_upper_key(key) && after.starts_with('[') {
            found.push(key);
        }
    }
    found
}

fn words(line: &str) -> Vec<&str> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| is_upper_key(w))
        .collect()
}

fn is_upper_key(key: &str) -> bool {
    valid_key(key)
        && key.len() >= 3
        && key.chars().any(|c| c.is_ascii_uppercase())
        && key.to_ascii_uppercase() == key
        && !is_framework_set(key)
}

pub fn failure_sentence(what: &str, keys: &[String]) -> String {
    let verb = if keys.len() == 1 { "is" } else { "are" };
    format!("The {what} failed: {} {verb} not set", keys.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_failed_build_names_its_keys_in_every_validator_shape() {
        let zod = lines(
            r#"Error: [
  {
    "code": "invalid_type",
    "path": ["NEXT_PUBLIC_APP_URL"],
    "message": "Required"
  }
]"#,
        );
        assert_eq!(keys_in_failure(&zod), vec!["NEXT_PUBLIC_APP_URL"]);

        let logged = lines("ZodError: issues: [ { path: [ 'STRIPE_KEY' ], message: 'Required' } ]");
        assert_eq!(keys_in_failure(&logged), vec!["STRIPE_KEY"]);

        let t3 = lines(
            "❌ Invalid environment variables: {\n  SMTP_HOST: [ 'Required' ],\n  STRIPE_KEY: [ 'Required' ]\n}\nerror: script \"build\" exited with code 1",
        );
        assert_eq!(keys_in_failure(&t3), vec!["SMTP_HOST", "STRIPE_KEY"]);

        let plain = lines(
            "Error: SENTRY_DSN is not set\nMissing environment variable: MAIL_FROM\nEnvironment variable API_KEY is required\nNODE_ENV is not set",
        );
        assert_eq!(
            keys_in_failure(&plain),
            vec!["SENTRY_DSN", "MAIL_FROM", "API_KEY"]
        );

        let quiet = lines(
            "Compiled successfully\nerror TS2307: Cannot find module './x'\nMissing semicolon at line 4",
        );
        assert!(
            keys_in_failure(&quiet).is_empty(),
            "{:?}",
            keys_in_failure(&quiet)
        );
    }

    #[test]
    fn the_failure_sentence_counts_its_keys() {
        assert_eq!(
            failure_sentence("build", &["NEXT_PUBLIC_APP_URL".into()]),
            "The build failed: NEXT_PUBLIC_APP_URL is not set"
        );
        assert_eq!(
            failure_sentence("build", &["A_KEY".into(), "B_KEY".into()]),
            "The build failed: A_KEY, B_KEY are not set"
        );
    }
}
