//! `lean-change outcome-map`: the section 'What was approved and what ran', built from a run's records, and its
//! place in an Accountable Review page. Pure: the caller supplies the show envelope, the intent and the repository's
//! tree identity. Nothing here reads a file, a process or Git.
use lean_change_provider::evaluate::old_problem;
use lean_change_provider::model::*;
use serde_json::Value;

const RAIL_TAG: &str = "<div class=\"rail-links\">";
const RAIL_LINK: &str =
    "<a data-rail=\"evidence\" href=\"#evidence\">What was approved and what ran</a>";
const UNKNOWN: &str = "unknown: not run";
const NO_OLD_CHECK: &str = "no old-vs-new check in this run";
const STALE: &str = "stale: ";
const CSS: &str = "\
#evidence { background: var(--surface, #ffffff); color: var(--ink, #1a1a1a); font-family: var(--body, system-ui, sans-serif); margin: 0 0 2rem; padding: 1rem 0; }
#evidence .om-title { font-size: 1.25rem; margin: 0 0 .5rem; }
#evidence h3 { font-size: 1.05rem; margin: 1.5rem 0 .25rem; }
#evidence p { margin: .35rem 0; }
#evidence .om-freshness, #evidence .om-summary { color: var(--muted, #666666); }
#evidence .om-goal { color: var(--ink-2, #333333); white-space: pre-wrap; }
#evidence details { border-top: 1px solid var(--rule, #d9d9d9); padding: .4rem 0; }
#evidence summary { cursor: pointer; }
#evidence .om-label { font-weight: 600; }
#evidence .om-id, #evidence .om-question, #evidence .om-disposition { font-family: var(--mono, ui-monospace, monospace); font-size: .85em; }
#evidence pre { background: var(--surface-2, #f4f4f4); border: 1px solid var(--rule, #d9d9d9); font-family: var(--mono, ui-monospace, monospace); font-size: .85em; margin: .3rem 0; overflow-wrap: anywhere; padding: .5rem; white-space: pre-wrap; }
";

/// The section for the run's latest intent. `current_tree` is the repository's tree identity now, or why it could not
/// be read.
pub fn section(
    show: &Value,
    input: &InitialInput,
    intent_id: &str,
    intent: &Intent,
    current_tree: &Result<String, String>,
) -> String {
    let context = show["result"]["context"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let verified = latest_verification(context, intent_id);
    let (freshness, stale) = freshness(verified.as_ref(), intent_id, current_tree);
    let rows: Vec<CriterionRow> = intent
        .criteria
        .iter()
        .map(|criterion| {
            let recorded = verified
                .as_ref()
                .and_then(|(_, v)| v.results.iter().find(|r| r.criterion == criterion.id));
            criterion_row(criterion, recorded, stale)
        })
        .collect();
    let findings = input.review.then(|| finding_rows(context));

    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for (criterion, row) in intent.criteria.iter().zip(&rows) {
        match groups.iter_mut().find(|(known, _)| *known == row.label) {
            Some((_, ids)) => ids.push(criterion.id.as_str()),
            None => groups.push((row.label.clone(), vec![criterion.id.as_str()])),
        }
    }
    let criteria_part: Vec<String> = groups
        .iter()
        .map(|(label, ids)| {
            let links: Vec<String> = ids.iter().map(|id| link(id)).collect();
            format!("{} {}: {}.", ids.len(), esc(label), links.join(", "))
        })
        .collect();
    let summary = match &findings {
        Some(rows) => format!(
            "Criteria: {} {}",
            criteria_part.join(" "),
            findings_part(rows)
        ),
        None => format!("Criteria: {}", criteria_part.join(" ")),
    };

    let mut html = format!(
        "<section id=\"evidence\" class=\"om\">\n<style>\n{CSS}</style>\n<h2 class=\"om-title\">What was approved and what ran</h2>\n<p class=\"om-freshness\">{}</p>\n<p class=\"om-goal\">{}</p>\n<p class=\"om-summary\">{summary}</p>\n",
        esc(&freshness),
        esc(&intent.goal),
    );
    for row in &rows {
        html.push_str(&row.html);
    }
    if let Some(findings) = &findings {
        html.push_str("<h3>Findings</h3>\n");
        for row in findings {
            html.push_str(&finding_html(row));
        }
    }
    html.push_str("</section>");
    html
}

/// A self-contained page: the section alone, titled with the run's label, or its id when it has none.
pub fn page(show: &Value, section: &str) -> String {
    let name = show["result"]["label"]
        .as_str()
        .filter(|label| !label.trim().is_empty())
        .or_else(|| show["result"]["run_id"].as_str())
        .unwrap_or_default();
    let title = if name.is_empty() {
        "Outcome map".to_owned()
    } else {
        format!("Outcome map: {name}")
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n</head>\n<body>\n<main>\n{section}\n</main>\n</body>\n</html>\n",
        esc(&title)
    )
}

/// The AR page with the section inserted after the first `</header>` inside `<main>`, and the rail link as the first
/// child of the rail-links div. Every other byte is kept. A missing anchor is an error that names it.
pub fn splice(ar_page: &str, section: &str) -> Result<String, String> {
    let main = ar_page
        .find("<main")
        .ok_or("--into: the page has no <main>")?;
    let header_end = ar_page[main..]
        .find("</header>")
        .map(|offset| main + offset + "</header>".len())
        .ok_or("--into: the page has no </header> after <main>")?;
    let rail_end = ar_page
        .find(RAIL_TAG)
        .map(|at| at + RAIL_TAG.len())
        .ok_or("--into: the page has no <div class=\"rail-links\">")?;
    let mut insertions = [(header_end, section), (rail_end, RAIL_LINK)];
    insertions.sort_by_key(|(at, _)| *at);
    let mut out = String::with_capacity(ar_page.len() + section.len() + RAIL_LINK.len());
    let mut from = 0;
    for (at, text) in insertions {
        out.push_str(&ar_page[from..at]);
        out.push_str(text);
        from = at;
    }
    out.push_str(&ar_page[from..]);
    Ok(out)
}

/// The latest verification of the intent, by sequence. A record that does not parse is skipped.
fn latest_verification(context: &[Value], intent_id: &str) -> Option<(String, Verification)> {
    context
        .iter()
        .filter(|r| r["kind"] == "verification" && r["data"]["intent"] == intent_id)
        .filter_map(|r| {
            let sequence = r["sequence"].as_u64()?;
            let id = r["id"].as_str()?;
            let verification = serde_json::from_value::<Verification>(r["data"].clone()).ok()?;
            Some((sequence, id.to_owned(), verification))
        })
        .max_by_key(|(sequence, _, _)| *sequence)
        .map(|(_, id, verification)| (id, verification))
}

/// The freshness paragraph, and whether the rows with a counted result carry the stale prefix.
fn freshness(
    verified: Option<&(String, Verification)>,
    intent_id: &str,
    current_tree: &Result<String, String>,
) -> (String, bool) {
    let Some((id, verification)) = verified else {
        return (format!("no verification for {intent_id}"), false);
    };
    match current_tree {
        Ok(tree) if *tree == verification.tree => (format!("verification current: {id}"), false),
        Ok(tree) => (
            format!(
                "verification stale: the repository tree is {tree}, but {id} verified {}.",
                verification.tree
            ),
            true,
        ),
        Err(reason) => (format!("verification freshness unknown: {reason}"), false),
    }
}

struct CriterionRow {
    label: String,
    html: String,
}

/// One criterion's row. A result counts only when its check is the approved check; otherwise the row is unknown.
fn criterion_row(
    criterion: &Criterion,
    recorded: Option<&CheckResult>,
    stale: bool,
) -> CriterionRow {
    let counted = recorded.filter(|r| r.check_sha256 == criterion.check.sha256());
    let (mut label, judgement) = match counted {
        Some(result) => judge(criterion, result),
        None => (UNKNOWN.to_owned(), None),
    };
    if stale && counted.is_some() {
        label.insert_str(0, STALE);
    }
    let argv = serde_json::to_string(&criterion.check.argv).expect("argv serializes");
    let mut body = format!("<p><b>Check:</b></p>\n<pre>{}</pre>\n", esc(&argv));
    match (recorded, counted) {
        (None, _) => body.push_str("<p><b>Current:</b> no result is recorded for this check.</p>\n"),
        (Some(_), None) => body.push_str(
            "<p><b>Current:</b> the recorded result is for another check, so it does not count.</p>\n",
        ),
        (Some(_), Some(result)) => body.push_str(&format!(
            "<p><b>Current:</b> {}.</p>\n<pre>{}</pre>\n",
            exit_text(result.timed_out, result.exit_code),
            esc(&result.output_tail)
        )),
    }
    match (&criterion.old, counted) {
        (None, _) => body.push_str(&format!("<p><b>Not compared:</b> {NO_OLD_CHECK}.</p>\n")),
        (Some(old), _) if old.expect == Expect::NotApplicable => body.push_str(&format!(
            "<p><b>Not compared:</b> {}</p>\n",
            esc(old.reason.as_deref().unwrap_or_default())
        )),
        (Some(old), Some(result)) => {
            body.push_str(&format!(
                "<p><b>Old tree</b> (expect {}):</p>\n",
                old.expect.name()
            ));
            if let Some(record) = result.old.as_ref().filter(|record| !record.runs.is_empty()) {
                for (index, run) in record.runs.iter().enumerate() {
                    body.push_str(&format!(
                        "<p><b>Old run {}:</b> {}.</p>\n<pre>{}</pre>\n",
                        index + 1,
                        exit_text(run.timed_out, run.exit_code),
                        esc(&run.output_tail)
                    ));
                }
            } else {
                body.push_str("<p>No old runs are recorded.</p>\n");
            }
            if let Some(problem) = judgement {
                body.push_str(&format!("<p><b>Judged:</b> {}</p>\n", esc(&problem)));
            }
        }
        (Some(_), None) => {}
    }
    let html = format!(
        "<details id=\"{id}\" class=\"om-criterion\">\n<summary><span class=\"om-id\">{id}</span> <span class=\"om-label\">{label}</span> <span class=\"om-statement\">{statement}</span></summary>\n{body}</details>\n",
        id = esc(&criterion.id),
        label = esc(&label),
        statement = esc(&criterion.statement),
    );
    CriterionRow { label, html }
}

/// The label of a counted result, and the old-run judge's words when the old runs do not show the expectation.
fn judge(criterion: &Criterion, result: &CheckResult) -> (String, Option<String>) {
    if result.timed_out || result.exit_code != Some(0) {
        return ("failed now".into(), None);
    }
    let Some(old) = &criterion.old else {
        return (format!("not compared: {NO_OLD_CHECK}"), None);
    };
    match (old.expect, old_problem(old, result)) {
        (Expect::NotApplicable, _) => (
            format!(
                "not compared: {}",
                old.reason.as_deref().unwrap_or_default()
            ),
            None,
        ),
        (expect, Some(problem)) => (
            format!("old tree did not match: expected {}", expect.name()),
            Some(problem),
        ),
        (Expect::Fails, None) => ("changed as intended (tested)".into(), None),
        (Expect::Same, None) => ("kept (same output before and after)".into(), None),
        (Expect::Absent, None) => ("new: nothing before to compare".into(), None),
    }
}

fn exit_text(timed_out: bool, exit_code: Option<i32>) -> String {
    match (timed_out, exit_code) {
        (true, _) => "timed out".into(),
        (false, Some(code)) => format!("exit {code}"),
        (false, None) => "no exit code".into(),
    }
}

/// A finding of an intent check or a review, with the latest disposition recorded for it.
struct FindingRow {
    id: String,
    finding: Finding,
    disposition: Option<Disposition>,
}

/// Every finding of every intent-check and review record, in context order.
fn finding_rows(context: &[Value]) -> Vec<FindingRow> {
    let dispositions: Vec<(u64, Disposition)> = context
        .iter()
        .filter(|r| r["kind"] == "finding-disposition")
        .filter_map(|r| {
            let sequence = r["sequence"].as_u64()?;
            let disposition = serde_json::from_value::<Disposition>(r["data"].clone()).ok()?;
            Some((sequence, disposition))
        })
        .collect();
    let mut rows = Vec::new();
    for record in context {
        let Some(record_id) = record["id"].as_str() else {
            continue;
        };
        for finding in findings_of(record) {
            let disposition = dispositions
                .iter()
                .filter(|(_, d)| d.review == record_id && d.finding == finding.id)
                .max_by_key(|(sequence, _)| *sequence)
                .map(|(_, d)| d.clone());
            rows.push(FindingRow {
                id: format!("{record_id}-{}", finding.id),
                finding,
                disposition,
            });
        }
    }
    rows
}

/// The findings of an intent-check or review record. Other records, and records that do not parse, have none.
fn findings_of(record: &Value) -> Vec<Finding> {
    let parsed = match record["kind"].as_str() {
        Some("intent-check") => {
            serde_json::from_value::<IntentCheck>(record["data"].clone()).map(|c| c.findings)
        }
        Some("review") => {
            serde_json::from_value::<Review>(record["data"].clone()).map(|r| r.findings)
        }
        _ => return Vec::new(),
    };
    parsed.unwrap_or_default()
}

fn disposition_name(row: &FindingRow) -> &'static str {
    match &row.disposition {
        None => "open",
        Some(d) => match d.disposition {
            DispositionKind::Fixed => "fixed",
            DispositionKind::Rejected => "rejected",
        },
    }
}

fn findings_part(rows: &[FindingRow]) -> String {
    let count = |state: &str| rows.iter().filter(|r| disposition_name(r) == state).count();
    let counts = format!(
        "Findings: {} ({} fixed, {} rejected, {} open).",
        rows.len(),
        count("fixed"),
        count("rejected"),
        count("open")
    );
    let links: Vec<String> = rows.iter().map(|r| link(&r.id)).collect();
    if links.is_empty() {
        counts
    } else {
        format!("{counts} {}", links.join(", "))
    }
}

fn finding_html(row: &FindingRow) -> String {
    let finding = &row.finding;
    let reason = row.disposition.as_ref().map_or(String::new(), |d| {
        format!("<p><b>Reason:</b> {}</p>\n", esc(&d.reason))
    });
    format!(
        "<details id=\"{id}\" class=\"om-finding\">\n<summary><span class=\"om-id\">{id}</span> <span class=\"om-question\">{question}</span> <span class=\"om-disposition\">{state}</span></summary>\n<p><b>Text:</b> {text}</p>\n<p><b>Evidence:</b> {evidence}</p>\n<p><b>Consequence:</b> {consequence}</p>\n{reason}</details>\n",
        id = esc(&row.id),
        question = esc(&finding.question),
        state = disposition_name(row),
        text = esc(&finding.text),
        evidence = esc(&finding.evidence),
        consequence = esc(&finding.consequence),
    )
}

fn link(id: &str) -> String {
    let id = esc(id);
    format!("<a href=\"#{id}\">{id}</a>")
}

/// Escapes text for an HTML element or a quoted attribute.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The tree the labelled verification was recorded on.
    const TREE: &str = "tree-recorded";

    fn check(id: &str) -> Value {
        json!({"argv": ["sh", "-c", format!("check {id}")], "timeout_ms": 1000})
    }

    fn criterion(id: &str, old: Option<Value>) -> Value {
        let mut value =
            json!({"id": id, "statement": format!("statement of {id}"), "check": check(id)});
        if let Some(old) = old {
            value["old"] = old;
        }
        value
    }

    fn expect(name: &str) -> Value {
        json!({"expect": name})
    }

    fn sha_of(criterion: &Value) -> String {
        serde_json::from_value::<Check>(criterion["check"].clone())
            .expect("check parses")
            .sha256()
    }

    fn run(exit_code: Option<i32>, output_tail: &str) -> Value {
        json!({"exit_code": exit_code, "timed_out": false, "output_tail": output_tail})
    }

    fn old_record(expect: &str, runs: Vec<Value>) -> Value {
        json!({"expect": expect, "runs": runs})
    }

    fn result(
        criterion: &Value,
        exit_code: Option<i32>,
        output_tail: &str,
        old: Option<Value>,
    ) -> Value {
        json!({"criterion": criterion["id"], "check_sha256": sha_of(criterion), "exit_code": exit_code, "timed_out": false, "output_tail": output_tail, "old": old})
    }

    fn timed_out(criterion: &Value) -> Value {
        let mut value = result(criterion, None, "slow", None);
        value["timed_out"] = json!(true);
        value
    }

    fn record(id: &str, kind: &str, sequence: u64, data: Value) -> Value {
        json!({"id": id, "kind": kind, "sequence": sequence, "data": data})
    }

    fn intent_record(goal: &str, criteria: Vec<Value>) -> Value {
        record(
            "intent-1",
            "intent",
            1,
            json!({"author": {"name": "owner", "kind": "human"}, "goal": goal, "constraints": [], "hard": false, "criteria": criteria}),
        )
    }

    fn verification_record(tree: &str, results: Vec<Value>) -> Value {
        record(
            "verification-2",
            "verification",
            2,
            json!({"intent": "intent-1", "tree": tree, "results": results}),
        )
    }

    /// A completed full show envelope. `review` adds principles (profile b); `old_vs_new` requires `old` per criterion.
    fn show(review: bool, old_vs_new: bool, context: Vec<Value>) -> Value {
        let mut input = json!({"schema_version": 1, "profile_version": "lean-test", "repo": "/nonexistent/outcome-map-test", "base": "base-sha", "review": review});
        if review {
            input["principles"] = json!({"source": "principles.md", "sha256": sha256_hex(b"principles"), "text": "principles"});
        }
        if old_vs_new {
            input["old_vs_new"] = json!({"proof_paths": []});
        }
        json!({"operation": "show", "status": "completed", "result": {"workflow_id": "lean-change", "run_id": "run-1", "label": "demo run", "initial_input": input, "context": context}})
    }

    fn render(show: &Value, tree: Result<String, String>) -> String {
        let (input, intent_id, intent) = crate::run::from_show(show).expect("show is valid");
        section(show, &input, &intent_id, &intent, &tree)
    }

    fn text_in<'a>(row: &'a str, class: &str) -> &'a str {
        let open = format!("<span class=\"{class}\">");
        let start = row.find(&open).expect("span exists") + open.len();
        &row[start..start + row[start..].find("</span>").expect("span closes")]
    }

    /// The text of one row, from its opening tag to its closing tag.
    fn row<'a>(html: &'a str, id: &str) -> &'a str {
        let start = html
            .find(&format!("<details id=\"{id}\" "))
            .expect("row exists");
        let end =
            start + html[start..].find("</details>").expect("row closes") + "</details>".len();
        &html[start..end]
    }

    fn label_of(html: &str, id: &str) -> String {
        text_in(row(html, id), "om-label").to_owned()
    }

    fn freshness_of(html: &str) -> &str {
        let open = "<p class=\"om-freshness\">";
        let start = html.find(open).expect("freshness paragraph") + open.len();
        &html[start..start + html[start..].find("</p>").expect("paragraph closes")]
    }

    /// Eleven criteria, one for each label and each way a row can lack a counted result, on an old-vs-new profile.
    fn labelled_criteria() -> Vec<Value> {
        vec![
            criterion("c1", Some(expect("fails"))),
            criterion("c2", Some(expect("same"))),
            criterion("c3", Some(expect("absent"))),
            criterion("c4", Some(expect("fails"))),
            criterion("c5", Some(expect("same"))),
            criterion("c6", Some(expect("absent"))),
            criterion(
                "c7",
                Some(json!({"expect": "none", "reason": "a guard over the gate"})),
            ),
            criterion("c8", Some(expect("fails"))),
            criterion("c9", Some(expect("fails"))),
            criterion("c10", Some(expect("absent"))),
            criterion("c11", Some(expect("absent"))),
        ]
    }

    /// The intent and a verification of `labelled_criteria` recorded on `tree`.
    fn labelled_context(tree: &str) -> Vec<Value> {
        let criteria = labelled_criteria();
        let mut wrong_sha = result(&criteria[10], Some(0), "ok", None);
        wrong_sha["check_sha256"] = json!("not-the-approved-check");
        let results = vec![
            result(
                &criteria[0],
                Some(0),
                "ok",
                Some(old_record(
                    "fails",
                    vec![run(Some(1), "no feature"), run(Some(1), "no feature")],
                )),
            ),
            result(
                &criteria[1],
                Some(0),
                "same",
                Some(old_record("same", vec![run(Some(0), "same")])),
            ),
            result(
                &criteria[2],
                Some(0),
                "added",
                Some(old_record("absent", vec![run(Some(1), "missing")])),
            ),
            result(
                &criteria[3],
                Some(0),
                "ok",
                Some(old_record(
                    "fails",
                    vec![run(Some(0), "x"), run(Some(0), "x")],
                )),
            ),
            result(
                &criteria[4],
                Some(0),
                "now",
                Some(old_record("same", vec![run(Some(0), "then")])),
            ),
            result(
                &criteria[5],
                Some(0),
                "ok",
                Some(old_record("absent", vec![run(Some(0), "x")])),
            ),
            result(
                &criteria[6],
                Some(0),
                "ok",
                Some(old_record("none", vec![])),
            ),
            result(&criteria[7], Some(1), "broke", None),
            timed_out(&criteria[8]),
            wrong_sha,
        ];
        vec![
            intent_record("Ship the thing.", criteria),
            verification_record(tree, results),
        ]
    }

    #[test]
    fn every_label_follows_the_judgement() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Ok(TREE.to_owned()),
        );
        let expected = [
            ("c1", "changed as intended (tested)"),
            ("c2", "kept (same output before and after)"),
            ("c3", "new: nothing before to compare"),
            ("c4", "old tree did not match: expected fails"),
            ("c5", "old tree did not match: expected same"),
            ("c6", "old tree did not match: expected absent"),
            ("c7", "not compared: a guard over the gate"),
            ("c8", "failed now"),
            ("c9", "failed now"),
            ("c10", "unknown: not run"),
            ("c11", "unknown: not run"),
        ];
        for (id, label) in expected {
            assert_eq!(label_of(&html, id), label, "row {id}");
        }
        assert!(row(&html, "c4").contains("old run 1 exited 0"));
        assert!(row(&html, "c1").contains("<pre>no feature</pre>"));
        assert!(row(&html, "c10").contains("no result is recorded"));
        assert!(row(&html, "c11").contains("another check"));
    }

    #[test]
    fn current_verification_names_its_record_and_adds_no_prefix() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Ok(TREE.to_owned()),
        );
        assert_eq!(freshness_of(&html), "verification current: verification-2");
        assert!(!html.contains("stale: "));
    }

    #[test]
    fn stale_tree_prefixes_every_row_with_a_result() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Ok("tree-later".to_owned()),
        );
        assert!(freshness_of(&html).starts_with("verification stale"));
        assert_eq!(label_of(&html, "c1"), "stale: changed as intended (tested)");
        assert_eq!(
            label_of(&html, "c2"),
            "stale: kept (same output before and after)"
        );
        assert_eq!(label_of(&html, "c8"), "stale: failed now");
        assert_eq!(label_of(&html, "c10"), "unknown: not run");
        assert_eq!(label_of(&html, "c11"), "unknown: not run");
        assert!(!html.contains("<span class=\"om-label\">changed as intended"));
        assert!(!html.contains("<span class=\"om-label\">kept (same"));
    }

    #[test]
    fn no_verification_leaves_every_row_unknown() {
        let html = render(
            &show(
                false,
                true,
                vec![intent_record("Ship the thing.", labelled_criteria())],
            ),
            Ok(TREE.to_owned()),
        );
        assert_eq!(freshness_of(&html), "no verification for intent-1");
        for id in ["c1", "c4", "c7", "c11"] {
            assert_eq!(label_of(&html, id), "unknown: not run", "row {id}");
        }
    }

    #[test]
    fn unreadable_tree_leaves_labels_unprefixed() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Err("repo /nonexistent/outcome-map-test: No such file or directory".to_owned()),
        );
        assert_eq!(
            freshness_of(&html),
            "verification freshness unknown: repo /nonexistent/outcome-map-test: No such file or directory"
        );
        assert_eq!(label_of(&html, "c1"), "changed as intended (tested)");
        assert_eq!(label_of(&html, "c8"), "failed now");
    }

    #[test]
    fn findings_show_each_disposition_with_latest_winning() {
        let finding = |id: &str, question: &str| json!({"id": id, "question": question, "text": format!("text of {id}"), "evidence": "evidence", "consequence": "consequence"});
        let reviewer = json!({"name": "reviewer", "kind": "agent"});
        let context = vec![
            intent_record("Ship the thing.", vec![criterion("c1", None)]),
            record(
                "intent-check-1",
                "intent-check",
                2,
                json!({"intent": "intent-1", "reviewer": reviewer, "fresh_context": true, "answers": [], "findings": [finding("f1", "use"), finding("f2", "edge"), finding("f3", "control")]}),
            ),
            record(
                "disp-1",
                "finding-disposition",
                3,
                json!({"review": "intent-check-1", "finding": "f1", "disposition": "rejected", "reason": "not in scope"}),
            ),
            record(
                "disp-2",
                "finding-disposition",
                4,
                json!({"review": "intent-check-1", "finding": "f1", "disposition": "fixed", "reason": "added the test"}),
            ),
            record(
                "disp-3",
                "finding-disposition",
                5,
                json!({"review": "intent-check-1", "finding": "f2", "disposition": "rejected", "reason": "the goal is silent"}),
            ),
            record(
                "review-1",
                "review",
                6,
                json!({"intent": "intent-1", "tree": "t", "reviewer": reviewer, "fresh_context": true, "answers": [], "findings": [finding("f1", "scope")]}),
            ),
            record(
                "disp-4",
                "finding-disposition",
                7,
                json!({"review": "review-1", "finding": "f1", "disposition": "fixed", "reason": "removed the extra file"}),
            ),
        ];
        let html = render(&show(true, false, context), Ok(TREE.to_owned()));
        assert!(html.contains("Findings: 4 (2 fixed, 1 rejected, 1 open)."));
        assert!(html.contains("<a href=\"#intent-check-1-f1\">intent-check-1-f1</a>"));
        let state = |id: &str| text_in(row(&html, id), "om-disposition").to_owned();
        assert_eq!(state("intent-check-1-f1"), "fixed");
        assert_eq!(state("intent-check-1-f2"), "rejected");
        assert_eq!(state("intent-check-1-f3"), "open");
        assert_eq!(state("review-1-f1"), "fixed");
        assert!(row(&html, "intent-check-1-f1").contains("<p><b>Reason:</b> added the test</p>"));
        assert!(row(&html, "intent-check-1-f1").contains("<p><b>Text:</b> text of f1</p>"));
        assert!(!row(&html, "intent-check-1-f1").contains("not in scope"));
        assert!(html.find("id=\"intent-check-1-f1\"") < html.find("id=\"review-1-f1\""));
    }

    #[test]
    fn profile_c_has_no_findings_part() {
        let context = vec![
            intent_record("Ship the thing.", vec![criterion("c1", None)]),
            record(
                "intent-check-1",
                "intent-check",
                2,
                json!({"intent": "intent-1", "reviewer": {"name": "r", "kind": "agent"}, "fresh_context": true, "answers": [], "findings": [{"id": "f1", "question": "use", "text": "t", "evidence": "e", "consequence": "c"}]}),
            ),
        ];
        let html = render(&show(false, false, context), Ok(TREE.to_owned()));
        assert!(!html.contains("Findings"));
        assert!(!html.contains("om-finding"));
        assert_eq!(label_of(&html, "c1"), "unknown: not run");
        assert!(html.contains(
            "<p class=\"om-summary\">Criteria: 1 unknown: not run: <a href=\"#c1\">c1</a>.</p>"
        ));
    }

    #[test]
    fn profile_c_without_old_runs_says_no_old_check_ran() {
        let context = vec![
            intent_record("Ship the thing.", vec![criterion("c1", None)]),
            verification_record(
                TREE,
                vec![result(&criterion("c1", None), Some(0), "ok", None)],
            ),
        ];
        let html = render(&show(false, false, context), Ok(TREE.to_owned()));
        assert_eq!(
            label_of(&html, "c1"),
            format!("not compared: {NO_OLD_CHECK}")
        );
        assert!(row(&html, "c1").contains(&format!("<p><b>Not compared:</b> {NO_OLD_CHECK}.</p>")));
    }

    #[test]
    fn standalone_page_loads_nothing_from_outside() {
        let section = render(
            &show(false, true, labelled_context(TREE)),
            Ok(TREE.to_owned()),
        );
        let page = page(&show(false, true, labelled_context(TREE)), &section);
        assert!(page.starts_with("<!doctype html>\n"));
        assert!(page.contains("<meta charset=\"utf-8\">"));
        assert!(page.contains("<title>Outcome map: demo run</title>"));
        assert!(page.contains("<main>\n<section id=\"evidence\""));
        for banned in ["<script", "<link", "@import", "src=", "url(", "http"] {
            assert!(!page.contains(banned), "page contains {banned}");
        }
    }

    #[test]
    fn record_text_is_escaped() {
        let context = vec![intent_record("Ship <b>&\"'", vec![criterion("c1", None)])];
        let html = render(&show(false, false, context), Ok(TREE.to_owned()));
        assert!(html.contains("<p class=\"om-goal\">Ship &lt;b&gt;&amp;&quot;&#39;</p>"));
        assert!(!html.contains("<b>&"));
    }

    #[test]
    fn section_is_deterministic() {
        let show = show(false, true, labelled_context(TREE));
        assert_eq!(
            render(&show, Ok(TREE.to_owned())),
            render(&show, Ok(TREE.to_owned()))
        );
    }

    const AR_PAGE: &str = "<!doctype html>\n<html>\n<body>\n<header>site</header>\n<main>\n<header class=\"masthead\">Title</header>\n<nav><div class=\"rail-links\">\n<a href=\"#01\">What changed</a>\n</div></nav>\n<section id=\"01\">What changed</section>\n</main>\n</body>\n</html>\n";
    const MARKER: &str = "<section id=\"evidence\" class=\"om\"></section>";

    #[test]
    fn into_inserts_two_strings_and_keeps_every_other_byte() {
        let spliced = splice(AR_PAGE, MARKER).expect("anchors exist");
        let expected = AR_PAGE
            .replacen("</header>\n<nav>", &format!("</header>{MARKER}\n<nav>"), 1)
            .replacen(RAIL_TAG, &format!("{RAIL_TAG}{RAIL_LINK}"), 1);
        assert_eq!(spliced, expected);
        let stripped = spliced.replacen(MARKER, "", 1).replacen(RAIL_LINK, "", 1);
        assert_eq!(stripped, AR_PAGE);
    }

    #[test]
    fn into_refuses_each_missing_anchor_by_name() {
        let no_main =
            "<html><body><header>x</header><div class=\"rail-links\"></div></body></html>";
        let error = splice(no_main, MARKER).expect_err("no main");
        assert!(error.contains("<main"), "{error}");
        let no_header = "<main><p>x</p><div class=\"rail-links\"></div></main>";
        let error = splice(no_header, MARKER).expect_err("no header");
        assert!(error.contains("</header>"), "{error}");
        let no_rail = "<main><header>x</header><p>y</p></main>";
        let error = splice(no_rail, MARKER).expect_err("no rail");
        assert!(error.contains("rail-links"), "{error}");
    }
}
