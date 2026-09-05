use crate::detect::env_hints::{self, SCHEMA_FILES, is_framework_set, is_managed, valid_key};

const ACCESSORS: [&str; 4] = [
    "process.env.",
    "Bun.env.",
    "import.meta.env.",
    "process.env[",
];
const DOTNET_ACCESSOR: &str = "Environment.GetEnvironmentVariable(";
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub key: String,
    pub path: String,
    pub optional: bool,
}

/// Every `process.env.X`-style read in one source file, plus the keys of an env schema file.
pub fn refs_in(path: &str, source: &str) -> Vec<Reference> {
    let mut refs: Vec<Reference> = Vec::new();
    let mut add = |key: &str, optional: bool| {
        if valid_key(key) && !refs.iter().any(|r| r.key == key) {
            refs.push(Reference {
                key: key.to_string(),
                path: path.to_string(),
                optional,
            });
        }
    };
    if SCHEMA_FILES.contains(&path) {
        for (key, optional) in env_hints::schema_keys(source) {
            add(&key, optional);
        }
    }
    for accessor in ACCESSORS {
        for (at, _) in source.match_indices(accessor) {
            let rest = &source[at + accessor.len()..];
            let rest = rest.trim_start_matches(['"', '\'', '`']);
            add(identifier(rest), false);
        }
    }
    for (at, _) in source.match_indices(DOTNET_ACCESSOR) {
        let rest = source[at + DOTNET_ACCESSOR.len()..].trim_start_matches('"');
        add(identifier(rest), false);
    }
    refs
}

fn identifier(text: &str) -> &str {
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    &text[..end]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub key: String,
    pub path: String,
    pub optional: bool,
}

/// The referenced keys neither the app nor Ferrum sets, one per key, sorted by name.
pub fn unset(referenced: &[Reference], stored: &[String], managed: &[String]) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();
    for r in referenced {
        if is_framework_set(&r.key)
            || is_managed(&r.key)
            || stored.contains(&r.key)
            || managed.contains(&r.key)
        {
            continue;
        }
        match findings.iter_mut().find(|f| f.key == r.key) {
            Some(existing) => existing.optional = existing.optional && r.optional,
            None => findings.push(Finding {
                key: r.key.clone(),
                path: r.path.clone(),
                optional: r.optional,
            }),
        }
    }
    findings.sort_by(|a, b| a.key.cmp(&b.key));
    findings
}

pub fn describe(findings: &[Finding]) -> String {
    let items: Vec<String> = findings
        .iter()
        .map(|f| {
            if f.optional {
                format!("{} ({}, optional)", f.key, f.path)
            } else {
                format!("{} ({})", f.key, f.path)
            }
        })
        .collect();
    format!("Referenced in the code but not set: {}", items.join(", "))
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

/// "The build failed: A is not set (src/env.ts)" with the file a hint knows, if any.
pub fn failure_sentence(what: &str, keys: &[(String, Option<String>)]) -> String {
    let named: Vec<String> = keys
        .iter()
        .map(|(key, file)| match file {
            Some(file) => format!("{key} ({file})"),
            None => key.clone(),
        })
        .collect();
    let verb = if keys.len() == 1 { "is" } else { "are" };
    format!("The {what} failed: {} {verb} not set", named.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn every_access_pattern_is_found_once_per_file() {
        let source = r#"
const a = process.env.SMTP_HOST;
const b = process.env["STRIPE_KEY"] ?? process.env.SMTP_HOST;
const c = Bun.env.LOG_LEVEL;
const d = import.meta.env.VITE_API;
const e = process.env.NODE_ENV;
"#;
        let found = refs_in("src/x.ts", source);
        let keys: Vec<&str> = found.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "SMTP_HOST",
                "STRIPE_KEY",
                "LOG_LEVEL",
                "VITE_API",
                "NODE_ENV"
            ]
        );
        let cs = refs_in(
            "Api/Program.cs",
            r#"var s = Environment.GetEnvironmentVariable("SENTRY_DSN");"#,
        );
        assert_eq!(cs[0].key, "SENTRY_DSN");
        let schema = refs_in("src/env.ts", "SMTP_HOST: z.string().optional(),\n");
        assert_eq!(
            schema,
            vec![Reference {
                key: "SMTP_HOST".into(),
                path: "src/env.ts".into(),
                optional: true
            }]
        );
    }

    #[test]
    fn unset_drops_what_is_set_or_managed_and_keeps_the_first_path() {
        let refs = vec![
            Reference {
                key: "SMTP_HOST".into(),
                path: "src/mail.ts".into(),
                optional: false,
            },
            Reference {
                key: "SMTP_HOST".into(),
                path: "src/env.ts".into(),
                optional: true,
            },
            Reference {
                key: "STRIPE_KEY".into(),
                path: "src/pay.ts".into(),
                optional: false,
            },
            Reference {
                key: "DATABASE_URL".into(),
                path: "src/db.ts".into(),
                optional: false,
            },
            Reference {
                key: "NODE_ENV".into(),
                path: "src/db.ts".into(),
                optional: false,
            },
            Reference {
                key: "REDIS_URL".into(),
                path: "src/q.ts".into(),
                optional: false,
            },
            Reference {
                key: "LOG_LEVEL".into(),
                path: "src/env.ts".into(),
                optional: true,
            },
        ];
        let found = unset(&refs, &["STRIPE_KEY".into()], &["REDIS_URL".into()]);
        assert_eq!(
            found,
            vec![
                Finding {
                    key: "LOG_LEVEL".into(),
                    path: "src/env.ts".into(),
                    optional: true
                },
                Finding {
                    key: "SMTP_HOST".into(),
                    path: "src/mail.ts".into(),
                    optional: false
                },
            ]
        );
        assert_eq!(
            describe(&found),
            "Referenced in the code but not set: LOG_LEVEL (src/env.ts, optional), SMTP_HOST (src/mail.ts)"
        );
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
    fn the_failure_sentence_names_the_file_when_a_hint_knows_it() {
        assert_eq!(
            failure_sentence(
                "build",
                &[("NEXT_PUBLIC_APP_URL".into(), Some("src/env.ts".into()))]
            ),
            "The build failed: NEXT_PUBLIC_APP_URL (src/env.ts) is not set"
        );
        assert_eq!(
            failure_sentence(
                "build",
                &[
                    ("A_KEY".into(), None),
                    ("B_KEY".into(), Some(".env.example".into()))
                ]
            ),
            "The build failed: A_KEY, B_KEY (.env.example) are not set"
        );
    }
}
