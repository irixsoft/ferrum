use ferrum_platform::ubuntu::{RULES_V4, RULES_V6};
use ferrum_platform::{Platform, PlatformError};
use std::path::Path;

/// Which of the host's own persisted rulesets exist, whether or not anything had to be opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Persisted {
    pub v4: bool,
    pub v6: bool,
}

impl Persisted {
    pub fn any(self) -> bool {
        self.v4 || self.v6
    }
}

/// The ports among `wanted` that a persisted INPUT chain rejects; `None` when nothing is rejected
/// or the file is ufw's own save.
pub fn blocked(rules: &str, wanted: &[u16]) -> Option<Vec<u16>> {
    if rules.contains("ufw-") {
        return None;
    }
    let mut accepted: Vec<u16> = Vec::new();
    for line in rules.lines().map(str::trim) {
        if !line.starts_with("-A INPUT") {
            continue;
        }
        if is_terminal(line) {
            let missing: Vec<u16> = wanted
                .iter()
                .copied()
                .filter(|p| !accepted.contains(p))
                .collect();
            return Some(missing);
        }
        if line.ends_with("-j ACCEPT") {
            accepted.extend(ports_of(line));
        }
    }
    None
}

/// Inserts one ACCEPT for `ports` immediately before the first REJECT or DROP in INPUT.
pub fn with_ports(rules: &str, ports: &[u16]) -> String {
    let list = ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let accept =
        format!("-A INPUT -p tcp -m state --state NEW -m multiport --dports {list} -j ACCEPT");
    let mut out = String::with_capacity(rules.len() + accept.len() + 1);
    let mut inserted = false;
    for line in rules.split_inclusive('\n') {
        let trimmed = line.trim();
        if !inserted && trimmed.starts_with("-A INPUT") && is_terminal(trimmed) {
            out.push_str(&accept);
            out.push('\n');
            inserted = true;
        }
        out.push_str(line);
    }
    out
}

/// Opens `ports` in both persisted rulesets and applies the ones that changed.
pub fn ensure_open(platform: &dyn Platform, ports: &[u16]) -> anyhow::Result<Persisted> {
    Ok(Persisted {
        v4: open_family(platform, RULES_V4, ports, |r| platform.iptables_restore(r))?,
        v6: open_family(platform, RULES_V6, ports, |r| platform.ip6tables_restore(r))?,
    })
}

fn open_family(
    platform: &dyn Platform,
    path: &str,
    ports: &[u16],
    restore: impl Fn(&str) -> Result<(), PlatformError>,
) -> anyhow::Result<bool> {
    let Some(rules) = platform.read_file(Path::new(path))? else {
        return Ok(false);
    };
    let Some(missing) = blocked(&rules, ports) else {
        return Ok(false);
    };
    if missing.is_empty() {
        return Ok(true);
    }
    let opened = with_ports(&rules, &missing);
    platform.write_file(Path::new(path), &opened, 0o644)?;
    restore(&opened)?;
    Ok(true)
}

/// Custom chains that the persisted OUTPUT chain jumps to, with their rules and the jumps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carry {
    pub chains: Vec<String>,
    pub rules: Vec<String>,
    pub jumps: Vec<String>,
}

/// What of an `iptables-save` filter table has to move into ufw for its OUTPUT protections to
/// survive; `None` when OUTPUT jumps to no custom chain.
pub fn carry(rules: &str) -> Option<Carry> {
    let filter = filter_table(rules);
    let custom: Vec<&str> = filter.iter().filter_map(|l| custom_chain(l)).collect();
    let jumps: Vec<&str> = filter
        .iter()
        .copied()
        .filter(|l| l.starts_with("-A OUTPUT ") && target(l).is_some_and(|t| custom.contains(&t)))
        .collect();
    let mut chains: Vec<&str> = Vec::new();
    let mut pending: Vec<&str> = jumps.iter().filter_map(|l| target(l)).collect();
    while let Some(chain) = pending.pop() {
        if chains.contains(&chain) {
            continue;
        }
        chains.push(chain);
        pending.extend(
            filter
                .iter()
                .filter(|l| in_chain(l, chain))
                .filter_map(|l| target(l))
                .filter(|t| custom.contains(t)),
        );
    }
    if chains.is_empty() {
        return None;
    }
    chains.sort_by_key(|c| custom.iter().position(|d| d == c));
    Some(Carry {
        rules: filter
            .iter()
            .filter(|l| chains.iter().any(|c| in_chain(l, c)))
            .map(|l| l.to_string())
            .collect(),
        chains: chains.into_iter().map(String::from).collect(),
        jumps: jumps.into_iter().map(String::from).collect(),
    })
}

/// Inserts `carry` into ufw's `before.rules` (or `before6.rules`) so its jumps are the first rules
/// of ufw's before-output chain; chains the file already declares are left as they are.
pub fn into_ufw(before_rules: &str, carry: &Carry) -> String {
    let filter = filter_table(before_rules);
    let declared: Vec<&str> = filter
        .iter()
        .filter_map(|l| l.strip_prefix(':')?.split_whitespace().next())
        .collect();
    let Some(output) = declared
        .iter()
        .find(|c| c.starts_with("ufw") && c.ends_with("-before-output"))
    else {
        return before_rules.to_string();
    };
    let fresh: Vec<&str> = carry
        .chains
        .iter()
        .map(String::as_str)
        .filter(|c| !declared.contains(c))
        .collect();
    if fresh.is_empty() {
        return before_rules.to_string();
    }
    let Some(last) = filter.iter().rposition(|l| l.starts_with(':')) else {
        return before_rules.to_string();
    };
    let mut block: Vec<String> = fresh.iter().map(|c| format!(":{c} - [0:0]")).collect();
    block.extend(
        carry
            .rules
            .iter()
            .filter(|l| fresh.iter().any(|c| in_chain(l, c)))
            .cloned(),
    );
    block.extend(
        carry
            .jumps
            .iter()
            .filter(|l| target(l).is_some_and(|t| fresh.contains(&t)))
            .filter_map(|l| l.strip_prefix("-A OUTPUT "))
            .map(|rest| format!("-A {output} {rest}")),
    );
    let anchor = filter[last];
    let mut out = String::with_capacity(before_rules.len());
    let mut table = "";
    let mut inserted = false;
    for line in before_rules.split_inclusive('\n') {
        out.push_str(line);
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_prefix('*') {
            table = name;
        }
        if !inserted && table == "filter" && trimmed == anchor {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            for l in &block {
                out.push_str(l);
                out.push('\n');
            }
            inserted = true;
        }
    }
    out
}

fn filter_table(rules: &str) -> Vec<&str> {
    let mut table = "";
    let mut lines = Vec::new();
    for line in rules.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('*') {
            table = name;
        } else if line == "COMMIT" {
            table = "";
        } else if table == "filter" && !line.is_empty() && !line.starts_with('#') {
            lines.push(line);
        }
    }
    lines
}

fn custom_chain(line: &str) -> Option<&str> {
    let mut words = line.strip_prefix(':')?.split_whitespace();
    let name = words.next()?;
    (words.next() == Some("-") && !name.starts_with("ufw")).then_some(name)
}

fn in_chain(line: &str, chain: &str) -> bool {
    line.strip_prefix("-A ")
        .and_then(|rest| rest.strip_prefix(chain))
        .is_some_and(|rest| rest.starts_with(' '))
}

fn target(line: &str) -> Option<&str> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let at = words.iter().rposition(|w| *w == "-j" || *w == "--jump")?;
    words.get(at + 1).copied()
}

fn is_terminal(line: &str) -> bool {
    line.ends_with("-j REJECT")
        || line.contains("-j REJECT ")
        || line.ends_with("-j DROP")
        || line.contains("-j DROP ")
}

fn ports_of(line: &str) -> Vec<u16> {
    let mut words = line.split_whitespace();
    while let Some(word) = words.next() {
        if word == "--dport" || word == "--dports" {
            return words
                .next()
                .unwrap_or_default()
                .split(',')
                .filter_map(|p| p.parse().ok())
                .collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    pub(in crate::security) const ORACLE: &str = r#"# CLOUD_IMG: This file was created/modified by the Cloud Image build process
# iptables configuration for Oracle Cloud Infrastructure

# See the Oracle-Provided Images section in the Oracle Cloud Infrastructure
# documentation for security impact of modifying or removing these rule

*filter
:INPUT ACCEPT [0:0]
:FORWARD ACCEPT [0:0]
:OUTPUT ACCEPT [463:49013]
:InstanceServices - [0:0]
-A INPUT -m state --state RELATED,ESTABLISHED -j ACCEPT
-A INPUT -p icmp -j ACCEPT
-A INPUT -i lo -j ACCEPT
-A INPUT -p udp --sport 123 -j ACCEPT
-A INPUT -p tcp -m state --state NEW -m tcp --dport 22 -j ACCEPT
-A INPUT -j REJECT --reject-with icmp-host-prohibited
-A FORWARD -j REJECT --reject-with icmp-host-prohibited
-A OUTPUT -d 169.254.0.0/16 -j InstanceServices
-A InstanceServices -d 169.254.0.2/32 -p tcp -m owner --uid-owner 0 -m tcp --dport 3260 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.2.0/24 -p tcp -m owner --uid-owner 0 -m tcp --dport 3260 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.4.0/24 -p tcp -m owner --uid-owner 0 -m tcp --dport 3260 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.5.0/24 -p tcp -m owner --uid-owner 0 -m tcp --dport 3260 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.0.2/32 -p tcp -m tcp --dport 80 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p udp -m udp --dport 53 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p tcp -m tcp --dport 53 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.0.3/32 -p tcp -m owner --uid-owner 0 -m tcp --dport 80 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.0.4/32 -p tcp -m tcp --dport 80 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p tcp -m tcp --dport 80 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p udp -m udp --dport 67 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p udp -m udp --dport 69 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.169.254/32 -p udp --dport 123 -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j ACCEPT
-A InstanceServices -d 169.254.0.0/16 -p tcp -m tcp -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j REJECT --reject-with tcp-reset
-A InstanceServices -d 169.254.0.0/16 -p udp -m udp -m comment --comment "See the Oracle-Provided Images section in the Oracle Cloud Infrastructure documentation for security impact of modifying or removing this rule" -j REJECT --reject-with icmp-port-unreachable
COMMIT
"#;

    pub(in crate::security) const BEFORE_RULES: &str = include_str!("testdata/before.rules");
    pub(in crate::security) const BEFORE6_RULES: &str = include_str!("testdata/before6.rules");

    const ORACLE_V6: &str = "# Generated by ip6tables-save v1.8.7 on Tue Sep  1 10:00:00 2026
*filter
:INPUT ACCEPT [0:0]
:FORWARD ACCEPT [0:0]
:OUTPUT ACCEPT [0:0]
-A INPUT -m state --state RELATED,ESTABLISHED -j ACCEPT
-A INPUT -p ipv6-icmp -j ACCEPT
-A INPUT -i lo -j ACCEPT
-A INPUT -p udp --sport 123 -j ACCEPT
-A INPUT -p tcp -m state --state NEW -m tcp --dport 22 -j ACCEPT
-A INPUT -j REJECT --reject-with icmp6-adm-prohibited
-A FORWARD -j REJECT --reject-with icmp6-adm-prohibited
COMMIT
";

    #[test]
    fn oracle_rejects_the_web_ports_but_not_ssh() {
        assert_eq!(blocked(ORACLE, &[22, 80, 443]), Some(vec![80, 443]));
        assert_eq!(blocked(ORACLE, &[22]), Some(vec![]));
        assert_eq!(blocked(ORACLE_V6, &[22, 80, 443]), Some(vec![80, 443]));
    }

    #[test]
    fn a_ruleset_without_a_terminal_rule_and_ufws_own_save_are_left_alone() {
        assert_eq!(
            blocked("*filter\n-A INPUT -i lo -j ACCEPT\nCOMMIT\n", &[80]),
            None
        );
        let ufw = "*filter\n-A INPUT -j ufw-before-input\n-A INPUT -j REJECT\nCOMMIT\n";
        assert_eq!(blocked(ufw, &[80]), None);
    }

    #[test]
    fn the_accept_lands_before_the_reject_and_nothing_else_moves() {
        let opened = with_ports(ORACLE, &[80, 443]);
        let accept =
            "-A INPUT -p tcp -m state --state NEW -m multiport --dports 80,443 -j ACCEPT\n";
        let reject = "-A INPUT -j REJECT --reject-with icmp-host-prohibited\n";
        assert_eq!(opened.matches(accept).count(), 1);
        assert!(opened.find(accept).unwrap() < opened.find(reject).unwrap());
        assert_eq!(opened.replacen(accept, "", 1), ORACLE);
        assert_eq!(blocked(&opened, &[22, 80, 443]), Some(vec![]));
        assert!(opened.contains("-m multiport --dports 80,443 -j ACCEPT\n-A INPUT -j REJECT"));
    }

    #[test]
    fn ensure_open_writes_restores_and_is_idempotent() {
        let p = FakePlatform::new();
        assert_eq!(ensure_open(&p, &[80, 443]).unwrap(), Persisted::default());
        assert!(p.calls_matching("iptables_").is_empty());
        assert!(p.calls_matching("ip6tables_").is_empty());
        assert!(p.calls_matching("write_file").is_empty());

        p.write_file(Path::new(RULES_V4), ORACLE, 0o644).unwrap();
        let opened = ensure_open(&p, &[2222, 80, 443]).unwrap();
        assert!(opened.v4 && !opened.v6);
        let written = p.written(RULES_V4).unwrap();
        assert!(
            written.contains("--dports 2222,80,443 -j ACCEPT"),
            "{written}"
        );
        assert_eq!(p.calls_matching("iptables_restore").len(), 1);
        assert!(p.calls_matching("ip6tables_").is_empty());
        assert!(p.written(RULES_V6).is_none());

        assert!(ensure_open(&p, &[2222, 80, 443]).unwrap().v4);
        assert_eq!(p.calls_matching("iptables_restore").len(), 1);
        assert_eq!(p.written(RULES_V4).unwrap(), written);
    }

    #[test]
    fn a_v6_ruleset_is_opened_through_ip6tables() {
        let p = FakePlatform::new();
        p.write_file(Path::new(RULES_V6), ORACLE_V6, 0o644).unwrap();
        let opened = ensure_open(&p, &[80, 443]).unwrap();
        assert!(!opened.v4 && opened.v6);
        assert!(opened.any());
        assert!(
            p.written(RULES_V6)
                .unwrap()
                .contains("--dports 80,443 -j ACCEPT\n-A INPUT -j REJECT --reject-with icmp6")
        );
        assert_eq!(p.calls_matching("ip6tables_restore").len(), 1);
        assert!(p.calls_matching("iptables_").is_empty());
    }

    #[test]
    fn oracle_carries_its_instance_services_chain() {
        let carry = carry(ORACLE).unwrap();
        assert_eq!(carry.chains, ["InstanceServices"]);
        assert_eq!(
            carry.jumps,
            ["-A OUTPUT -d 169.254.0.0/16 -j InstanceServices"]
        );
        assert_eq!(carry.rules.len(), 15);
        assert!(
            carry
                .rules
                .iter()
                .all(|r| r.starts_with("-A InstanceServices -d 169.254."))
        );
        assert!(carry.rules[0].starts_with(
            "-A InstanceServices -d 169.254.0.2/32 -p tcp -m owner --uid-owner 0 -m tcp --dport 3260 "
        ));
        assert!(carry.rules[14].ends_with("-j REJECT --reject-with icmp-port-unreachable"));
    }

    #[test]
    fn a_ruleset_without_output_jumps_carries_nothing() {
        assert_eq!(carry(ORACLE_V6), None);
        let rules =
            "*filter\n:OUTPUT ACCEPT [0:0]\n:Unused - [0:0]\n-A OUTPUT -o lo -j ACCEPT\nCOMMIT\n";
        assert_eq!(carry(rules), None);
        let nat =
            "*nat\n:Nat - [0:0]\n-A OUTPUT -j Nat\nCOMMIT\n*filter\n:OUTPUT ACCEPT [0:0]\nCOMMIT\n";
        assert_eq!(carry(nat), None);
    }

    #[test]
    fn every_chain_output_reaches_is_carried() {
        let rules = "*filter
:OUTPUT ACCEPT [0:0]
:First - [0:0]
:Second - [0:0]
:Nested - [0:0]
-A OUTPUT -d 10.0.0.0/8 -j First
-A OUTPUT -p udp -j Second
-A First -j Nested
-A Second -j DROP
-A Nested -j ACCEPT
COMMIT
";
        let carry = carry(rules).unwrap();
        assert_eq!(carry.chains, ["First", "Second", "Nested"]);
        assert_eq!(carry.jumps.len(), 2);
        assert_eq!(
            carry.rules,
            [
                "-A First -j Nested",
                "-A Second -j DROP",
                "-A Nested -j ACCEPT"
            ]
        );
    }

    #[test]
    fn the_jump_becomes_the_first_rule_of_ufws_before_output() {
        let carried = into_ufw(BEFORE_RULES, &carry(ORACLE).unwrap());
        let lines: Vec<&str> = carried.lines().collect();
        let at = lines
            .iter()
            .position(|l| *l == ":ufw-not-local - [0:0]")
            .unwrap();
        assert_eq!(lines[at + 1], ":InstanceServices - [0:0]");
        assert!(lines[at + 2].starts_with("-A InstanceServices -d 169.254.0.2/32 "));
        assert_eq!(
            lines[at + 17],
            "-A ufw-before-output -d 169.254.0.0/16 -j InstanceServices"
        );
        assert_eq!(lines[at + 18], "# End required lines");
        let first_output = lines
            .iter()
            .find(|l| l.starts_with("-A ufw-before-output "))
            .unwrap();
        assert_eq!(
            *first_output,
            "-A ufw-before-output -d 169.254.0.0/16 -j InstanceServices"
        );
        assert_eq!(carried.lines().count(), BEFORE_RULES.lines().count() + 17);
        let removed: String = carried
            .lines()
            .filter(|l| !l.contains("InstanceServices"))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(removed, BEFORE_RULES);
    }

    #[test]
    fn before6_uses_ufw6s_output_chain() {
        let v6 = "*filter\n:OUTPUT ACCEPT [0:0]\n:Meta - [0:0]\n-A OUTPUT -d fd00::/8 -j Meta\n-A Meta -j ACCEPT\nCOMMIT\n";
        let carried = into_ufw(BEFORE6_RULES, &carry(v6).unwrap());
        let lines: Vec<&str> = carried.lines().collect();
        let at = lines
            .iter()
            .position(|l| *l == ":ufw6-before-forward - [0:0]")
            .unwrap();
        assert_eq!(
            lines[at + 1..at + 4],
            [
                ":Meta - [0:0]",
                "-A Meta -j ACCEPT",
                "-A ufw6-before-output -d fd00::/8 -j Meta"
            ]
        );
    }

    #[test]
    fn carrying_twice_changes_nothing() {
        let carry = carry(ORACLE).unwrap();
        let once = into_ufw(BEFORE_RULES, &carry);
        assert_ne!(once, BEFORE_RULES);
        assert_eq!(into_ufw(&once, &carry), once);
    }
}
