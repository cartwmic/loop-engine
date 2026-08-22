use std::collections::BTreeMap;
use std::path::Path;

use crate::config::RepoConfig;
use crate::git;
use crate::prd::{parse_prd, Prd, RecordKind};

/// Return the PRD immediately before the current working-tree PRD.
///
/// The checker has two callers: a gate normally checks a committed `HEAD`,
/// while a local candidate may be an uncommitted working-tree edit.  When the
/// working-tree bytes still equal `HEAD`, the preceding version is `HEAD^`.
/// When the PRD is dirty, `HEAD` is the preceding committed version.  No
/// older commit is considered.
///
/// A missing committed PRD is the first-adoption case and returns `None`.  A
/// present but malformed committed PRD is an error rather than an invitation
/// to promote an older version into authority.
pub(crate) fn preceding_committed_prd(
    repo: &Path,
    cfg: &RepoConfig,
    current_text: &str,
) -> Result<Option<Prd>, String> {
    let Some(head) = git::head_commit(repo)? else {
        return Ok(None);
    };

    let head_text = git::show_blob(repo, &head, &cfg.prd)?;
    let baseline = if head_text.as_deref() == Some(current_text) {
        git::first_parent(repo, &head)?
    } else {
        Some(head)
    };

    let Some(commit) = baseline else {
        return Ok(None);
    };
    let Some(text) = git::show_blob(repo, &commit, &cfg.prd)? else {
        return Ok(None);
    };
    match parse_prd(&text) {
        Ok(prd) => Ok(Some(prd)),
        Err(errors) => Err(format!(
            "immediately preceding committed PRD is malformed: {}",
            errors.join("; ")
        )),
    }
}

/// Check the one-step continuity contract for two parsed PRDs.
///
/// A live record keeps its exact ID/title pair while live, or becomes a
/// tombstone with that exact pair when retired.  Tombstones are retained
/// forever and can never become live again.  New IDs are allowed; there is no
/// history scan for IDs outside this immediate pair.
pub(crate) fn continuity_findings(current: &Prd, previous: Option<&Prd>) -> Vec<String> {
    let Some(previous) = previous else {
        return Vec::new();
    };

    let current_by_id: BTreeMap<&str, (&str, &RecordKind)> = current
        .records
        .iter()
        .map(|record| (record.id.as_str(), (record.title.as_str(), &record.kind)))
        .collect();
    let mut findings = Vec::new();

    for old in &previous.records {
        let Some((title, kind)) = current_by_id.get(old.id.as_str()).copied() else {
            let message = match old.kind {
                RecordKind::Live { .. } => format!(
                    "previously live ID {} disappeared; retire it with an exact-title tombstone",
                    old.id
                ),
                RecordKind::Tombstone => {
                    format!("retained tombstone {} disappeared", old.id)
                }
            };
            findings.push(message);
            continue;
        };

        if title != old.title {
            findings.push(format!(
                "ID {} changed title from {:?} to {:?}; ID plus title is the requirement identity",
                old.id, old.title, title
            ));
            continue;
        }

        if let (RecordKind::Tombstone, RecordKind::Live { .. }) = (&old.kind, kind) {
            findings.push(format!(
                "tombstoned ID {} was revived; retained tombstones cannot become live",
                old.id
            ));
        }
        // A live record may remain live or retire as its exact-title
        // tombstone.  A retained tombstone may remain a tombstone.
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prd::parse_prd;

    fn live(id: &str, title: &str) -> Prd {
        parse_prd(&format!(
            "### {id}: {title}\n- Status: live\n- Coverage: e2e/journey\n"
        ))
        .unwrap()
    }

    fn tombstone(id: &str, title: &str) -> Prd {
        parse_prd(&format!("### {id}: {title}\n- Status: tombstone\n")).unwrap()
    }

    #[test]
    fn first_adoption_has_no_findings() {
        assert!(continuity_findings(&live("LE-1", "A"), None).is_empty());
    }

    #[test]
    fn exact_id_and_title_stays_live() {
        let previous = live("LE-1", "A");
        assert!(continuity_findings(&live("LE-1", "A"), Some(&previous)).is_empty());
    }

    #[test]
    fn title_change_is_reassignment() {
        let previous = live("LE-1", "A");
        let findings = continuity_findings(&live("LE-1", "B"), Some(&previous));
        assert!(findings
            .iter()
            .any(|finding| finding.contains("changed title")));
    }

    #[test]
    fn live_requirement_can_retire_with_exact_tombstone() {
        let previous = live("LE-1", "A");
        assert!(continuity_findings(&tombstone("LE-1", "A"), Some(&previous)).is_empty());
    }

    #[test]
    fn live_requirement_cannot_disappear() {
        let previous = live("LE-1", "A");
        let current = live("LE-2", "B");
        let findings = continuity_findings(&current, Some(&previous));
        assert!(findings.iter().any(|finding| finding.contains("LE-1")));
    }

    #[test]
    fn tombstone_cannot_disappear_or_revive() {
        let previous = tombstone("LE-1", "A");
        let missing = live("LE-2", "B");
        let findings = continuity_findings(&missing, Some(&previous));
        assert!(findings
            .iter()
            .any(|finding| finding.contains("disappeared")));

        let revived = live("LE-1", "A");
        let findings = continuity_findings(&revived, Some(&previous));
        assert!(findings.iter().any(|finding| finding.contains("revived")));
    }
}
