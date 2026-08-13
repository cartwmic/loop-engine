use crate::config::DeterministicPolicy;
use crate::document::{
    command_in_section, heading_match, headings, references, resolve_reference, Snapshot,
};
use serde_json::json;

pub fn evaluate(snapshot: &Snapshot, policies: &[DeterministicPolicy]) -> Vec<serde_json::Value> {
    let mut findings = Vec::new();
    let hs = headings(&snapshot.text);
    for policy in policies {
        let violated = match policy {
            DeterministicPolicy::NonEmpty { .. } => snapshot.text.trim().is_empty(),
            DeterministicPolicy::RequiredHeading { aliases, level, .. } => !hs.iter().any(|h| {
                heading_match(&h.title, aliases) && level.is_none_or(|wanted| h.level == wanted)
            }),
            DeterministicPolicy::AnyHeading { level, .. } => !hs.iter().any(|h| h.level == *level),
            DeterministicPolicy::CommandInSection {
                section_aliases, ..
            } => !command_in_section(&snapshot.text, section_aliases),
            DeterministicPolicy::LocalReferencesResolve { .. } => false,
        };
        if violated {
            findings.push(json!({"policy_id": policy.id(), "message": message(policy)}));
        }
        if matches!(policy, DeterministicPolicy::LocalReferencesResolve { .. }) {
            for reference in references(&snapshot.text) {
                match resolve_reference(
                    snapshot.path.parent().unwrap_or(snapshot.path.as_path()),
                    &reference,
                ) {
                    Ok(path) if path.as_os_str().is_empty() => {}
                    Ok(_) => {}
                    Err(message) => {
                        findings.push(json!({"policy_id": policy.id(), "message": message}))
                    }
                }
            }
        }
    }
    findings
}
fn message(policy: &DeterministicPolicy) -> String {
    match policy {
        DeterministicPolicy::NonEmpty { .. } => {
            "document must contain non-whitespace content".into()
        }
        DeterministicPolicy::RequiredHeading { aliases, .. } => {
            format!("required heading missing (aliases: {})", aliases.join(", "))
        }
        DeterministicPolicy::AnyHeading { level, .. } => {
            format!("any level-{level} heading required")
        }
        DeterministicPolicy::CommandInSection {
            section_aliases, ..
        } => format!(
            "section must contain executable command (aliases: {})",
            section_aliases.join(", ")
        ),
        DeterministicPolicy::LocalReferencesResolve { .. } => {
            "local reference does not resolve".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn returns_all_violations_in_profile_order() {
        let snapshot = Snapshot {
            target_id: "doc".into(),
            path: PathBuf::from("/tmp/doc.md"),
            text: "text".into(),
            sha256: "a".repeat(64),
        };
        let policies = vec![
            DeterministicPolicy::AnyHeading {
                id: "title".into(),
                level: 1,
            },
            DeterministicPolicy::RequiredHeading {
                id: "purpose".into(),
                aliases: vec!["Purpose".into()],
                level: None,
            },
            DeterministicPolicy::CommandInSection {
                id: "command".into(),
                section_aliases: vec!["Setup".into()],
            },
        ];
        let findings = evaluate(&snapshot, &policies);
        assert_eq!(
            findings
                .iter()
                .map(|finding| finding["policy_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["title", "purpose", "command"]
        );
    }

    #[test]
    fn fenced_and_empty_headings_do_not_satisfy_policies() {
        let snapshot = Snapshot {
            target_id: "doc".into(),
            path: PathBuf::from("/tmp/doc.md"),
            text: "```md\n# Fake\n## Purpose\n```\n# ###\n".into(),
            sha256: "a".repeat(64),
        };
        let policies = vec![
            DeterministicPolicy::AnyHeading {
                id: "title".into(),
                level: 1,
            },
            DeterministicPolicy::RequiredHeading {
                id: "purpose".into(),
                aliases: vec!["Purpose".into()],
                level: None,
            },
        ];
        assert_eq!(evaluate(&snapshot, &policies).len(), 2);
    }

    #[test]
    fn both_shipped_profiles_accept_supported_carriers() {
        let cases = [
            (
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/readme.json")),
                "# Product\n## Overview\nPurpose.\n## Quick Start\n```sh\ninstall\n```\n## Examples\nUse.\n## Tests\n```sh\ntest\n```\n",
            ),
            (
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/data/agents.json")),
                "## Authority\nRules.\n## Development Workflow\n```sh\ntest\n```\n## Handoff\nReport evidence.\n",
            ),
        ];
        for (raw, text) in cases {
            let mut value: serde_json::Value = serde_json::from_str(raw).unwrap();
            value["target"]["path"] = "/tmp/doc.md".into();
            let config = crate::config::InitialInput::parse(&value).unwrap();
            let snapshot = Snapshot {
                target_id: config.target.id,
                path: PathBuf::from("/tmp/doc.md"),
                text: text.into(),
                sha256: "a".repeat(64),
            };
            assert!(evaluate(&snapshot, &config.deterministic_policies).is_empty());
        }
    }
}
