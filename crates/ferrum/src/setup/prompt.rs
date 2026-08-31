use anyhow::{Context, bail};
use std::io::{BufRead, IsTerminal, Write};

pub fn validate_hostname(s: &str) -> Result<String, String> {
    let raw = s.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return Err("Enter a hostname, for example panel.example.com".into());
    }
    if raw.contains("://") || raw.contains('/') {
        return Err("Enter the hostname on its own, with no scheme and no path".into());
    }
    if raw.parse::<std::net::IpAddr>().is_ok() {
        return Err(
            "Ferrum needs a domain, not an IP address — a passkey cannot be enrolled against one"
                .into(),
        );
    }

    let host = raw.trim_end_matches('.').to_string();
    if !host.contains('.') {
        return Err(format!(
            "\"{host}\" is a single label; enter a full domain such as panel.example.com"
        ));
    }
    if host.len() > 253 {
        return Err("That hostname is longer than DNS allows".into());
    }
    for label in host.split('.') {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !valid {
            return Err(format!("\"{host}\" is not a valid hostname"));
        }
    }
    Ok(host)
}

pub fn validate_email(s: &str) -> Result<String, String> {
    let email = s.trim().to_string();
    if email.chars().any(char::is_whitespace) {
        return Err("An email address cannot contain spaces".into());
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Enter an email address, for example you@example.com".into());
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err("Enter an email address, for example you@example.com".into());
    }
    if domain.starts_with('.') || domain.ends_with('.') || email.matches('@').count() != 1 {
        return Err("Enter an email address, for example you@example.com".into());
    }
    Ok(email)
}

pub fn require_terminal() -> anyhow::Result<()> {
    if std::io::stdin().is_terminal() {
        return Ok(());
    }
    bail!(
        "Setup needs a terminal to ask three questions, and none is attached.\nRun `ferrum setup` from a shell, or pass --non-interactive with --hostname and --email."
    )
}

pub fn ask(
    question: &str,
    hint: &str,
    validate: impl Fn(&str) -> Result<String, String>,
) -> anyhow::Result<String> {
    let mut stdin = std::io::stdin().lock();
    loop {
        print!("  {question} ");
        if !hint.is_empty() {
            print!("({hint}) ");
        }
        std::io::stdout().flush().ok();

        let mut line = String::new();
        let read = stdin
            .read_line(&mut line)
            .context("reading from terminal")?;
        if read == 0 {
            bail!("The terminal closed before setup finished.");
        }
        match validate(&line) {
            Ok(v) => return Ok(v),
            Err(e) => println!("  {e}\n"),
        }
    }
}

pub fn confirm(question: &str, default: bool) -> anyhow::Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    let answer = ask(question, hint, |s| {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "" | "y" | "yes" | "n" | "no" => Ok(s),
            _ => Err("Answer yes or no".into()),
        }
    })?;
    Ok(match answer.as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_normal_hostname() {
        assert_eq!(
            validate_hostname(" Panel.Example.COM ").unwrap(),
            "panel.example.com"
        );
    }

    #[test]
    fn rejects_an_ip_address_with_a_reason() {
        let e = validate_hostname("203.0.113.10").unwrap_err();
        assert!(e.contains("domain"), "{e}");
    }

    #[test]
    fn rejects_a_bare_label() {
        assert!(validate_hostname("panel").is_err());
    }

    #[test]
    fn rejects_a_url() {
        assert!(validate_hostname("https://panel.example.com").is_err());
    }

    #[test]
    fn accepts_a_fully_qualified_name_with_a_trailing_dot() {
        assert_eq!(
            validate_hostname("panel.example.com.").unwrap(),
            "panel.example.com"
        );
    }

    #[test]
    fn rejects_labels_with_illegal_characters() {
        assert!(validate_hostname("pa nel.example.com").is_err());
        assert!(validate_hostname("-panel.example.com").is_err());
        assert!(validate_hostname("panel..example.com").is_err());
    }

    #[test]
    fn accepts_and_trims_an_email() {
        assert_eq!(
            validate_email("  me@example.com ").unwrap(),
            "me@example.com"
        );
    }

    #[test]
    fn rejects_an_email_without_an_at() {
        assert!(validate_email("nope").is_err());
    }

    #[test]
    fn rejects_an_email_without_a_domain_dot() {
        assert!(validate_email("me@localhost").is_err());
    }
}
