//! `lean-change outcome-map`: the section 'What was approved and what ran', built from a run's records, and its
//! place in an Accountable Review page. Pure: the caller supplies the show envelope, the intent and the repository's
//! tree identity. Nothing here reads a file, a process or Git.
use lean_change_provider::evaluate::old_problem;
use lean_change_provider::model::*;
use serde_json::Value;

const RAIL_TAG: &str = "<div class=\"rail-links\">";
const RAIL_LINK: &str =
    "<a data-rail=\"evidence\" href=\"#evidence\">What was approved and what ran</a>";
const UNKNOWN: &str = "not run yet";
const NO_OLD_CHECK: &str = "this run has no old-code check";
const STALE: &str = "out of date: ";
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
    let latest = latest_verification(context, intent_id);
    let (freshness, stale) = freshness(latest.as_ref(), intent_id, current_tree);
    let verified = latest.and_then(|(id, parsed)| parsed.ok().map(|v| (id, v)));
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
    let checks = intent.criteria.len();
    let mut sentences = vec![if checks == 1 {
        "1 check was approved.".to_owned()
    } else {
        format!("{checks} checks were approved.")
    }];
    for (label, ids) in &groups {
        let links: Vec<String> = ids.iter().map(|id| link(id)).collect();
        sentences.push(format!(
            "{} {}: {}.",
            ids.len(),
            esc(label),
            links.join(", ")
        ));
    }
    if let Some(findings) = &findings {
        sentences.push(findings_sentence(findings));
    }
    let summary = sentences.join(" ");

    let mut html = format!(
        "<section id=\"evidence\" class=\"om\">\n<style>\n{CSS}</style>\n<h2 class=\"om-title\">What was approved and what ran</h2>\n<p class=\"om-freshness\">{}</p>\n<p class=\"om-goal\"><b>The approved goal, as written:</b> {}</p>\n<p class=\"om-summary\">{summary}</p>\n",
        esc(&freshness),
        esc(&intent.goal),
    );
    for row in &rows {
        html.push_str(&row.html);
    }
    if let Some(findings) = &findings {
        html.push_str("<h3>Issues raised, and how each was settled</h3>\n");
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
    let main_end = ar_page[main..]
        .find("</main>")
        .map_or(ar_page.len(), |offset| main + offset);
    let header_end = ar_page[main..main_end]
        .find("</header>")
        .map(|offset| main + offset + "</header>".len())
        .ok_or("--into: the page has no </header> inside <main>")?;
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

/// The latest verification of the intent, by sequence, and its data if it parses. An unreadable latest record is
/// returned as an error, never replaced by an older one.
fn latest_verification(
    context: &[Value],
    intent_id: &str,
) -> Option<(String, Result<Verification, String>)> {
    context
        .iter()
        .filter(|r| r["kind"] == "verification" && r["data"]["intent"] == intent_id)
        .max_by_key(|r| r["sequence"].as_u64())
        .map(|r| {
            let id = r["id"].as_str().unwrap_or("verification").to_owned();
            let parsed = serde_json::from_value::<Verification>(r["data"].clone())
                .map_err(|e| e.to_string());
            (id, parsed)
        })
}

/// The freshness paragraph, and whether the rows with a recorded result carry the stale prefix.
fn freshness(
    latest: Option<&(String, Result<Verification, String>)>,
    intent_id: &str,
    current_tree: &Result<String, String>,
) -> (String, bool) {
    let Some((id, parsed)) = latest else {
        return (
            format!("The checks have not run yet for the latest goal ({intent_id})."),
            false,
        );
    };
    let verification = match parsed {
        Ok(verification) => verification,
        Err(error) => {
            return (
                format!("The last run of the checks ({id}) could not be read: {error}"),
                false,
            )
        }
    };
    match current_tree {
        Ok(tree) if *tree == verification.tree => (
            format!("These results are current: the last run of the checks ({id}) used the code as it is now."),
            false,
        ),
        Ok(tree) => (
            format!(
                "These results are out of date: the code changed after the last run of the checks ({id}). Code fingerprint now {tree}; when checked {}.",
                verification.tree
            ),
            true,
        ),
        Err(reason) => (
            format!("It is unknown whether these results are current: {reason}"),
            false,
        ),
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
    if stale && recorded.is_some() {
        label.insert_str(0, STALE);
    }
    let argv = serde_json::to_string(&criterion.check.argv).expect("argv serializes");
    let mut body = format!(
        "<p><b>Command that checks it:</b></p>\n<pre>{}</pre>\n",
        esc(&argv)
    );
    match recorded {
        None => {
            body.push_str("<p><b>On the new code:</b> no result is recorded for this check.</p>\n")
        }
        Some(result) => {
            if counted.is_none() {
                body.push_str(&format!(
                    "<p>This result was recorded for a different check command (sha256 {}), so it does not count. It is shown as recorded.</p>\n",
                    esc(&result.check_sha256)
                ));
            }
            body.push_str(&format!(
                "<p><b>On the new code:</b> {}</p>\n<pre>{}</pre>\n",
                run_text(result.timed_out, result.exit_code),
                esc(&result.output_tail)
            ));
        }
    }
    match (&criterion.old, recorded) {
        (None, _) => body.push_str(&format!(
            "<p><b>Not run on the old code:</b> {NO_OLD_CHECK}.</p>\n"
        )),
        (Some(old), _) if old.expect == Expect::NotApplicable => body.push_str(&format!(
            "<p><b>Not run on the old code:</b> {}</p>\n",
            esc(old.reason.as_deref().unwrap_or_default())
        )),
        (Some(old), recorded) => {
            body.push_str(&format!(
                "<p><b>On the old code ({}):</b></p>\n",
                old_expectation_text(old.expect)
            ));
            if let Some(reason) = &old.reason {
                body.push_str(&format!("<p><b>Reason given:</b> {}</p>\n", esc(reason)));
            }
            if let Some(result) = recorded {
                if let Some(record) = result.old.as_ref().filter(|record| !record.runs.is_empty()) {
                    for (index, run) in record.runs.iter().enumerate() {
                        body.push_str(&format!(
                            "<p><b>Old code, run {}:</b> {}</p>\n<pre>{}</pre>\n",
                            index + 1,
                            run_text(run.timed_out, run.exit_code),
                            esc(&run.output_tail)
                        ));
                    }
                } else {
                    body.push_str("<p>No runs on the old code are recorded.</p>\n");
                }
                if let Some(problem) = judgement {
                    body.push_str(&format!(
                        "<p><b>Why the old code did not match:</b> {}</p>\n",
                        esc(&problem)
                    ));
                }
            }
        }
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
        return ("fails on the new code".into(), None);
    }
    let Some(old) = &criterion.old else {
        return (format!("not run on the old code: {NO_OLD_CHECK}"), None);
    };
    match (old.expect, old_problem(old, result)) {
        (Expect::NotApplicable, _) => (
            format!(
                "not run on the old code: {}",
                old.reason.as_deref().unwrap_or_default()
            ),
            None,
        ),
        (Expect::Fails, Some(problem)) => (
            "old code did not behave as expected: expected it to fail".into(),
            Some(problem),
        ),
        (Expect::Same, Some(problem)) => (
            "old code did not behave as expected: expected the same output".into(),
            Some(problem),
        ),
        (Expect::Absent, Some(problem)) => (
            "old code did not behave as expected: expected the feature to be missing".into(),
            Some(problem),
        ),
        (Expect::Fails, None) => (
            "changed as intended: failed on the old code, passes now".into(),
            None,
        ),
        (Expect::Same, None) => ("kept: same output on the old and new code".into(), None),
        (Expect::Absent, None) => ("new: the old code did not have this".into(), None),
    }
}

/// What one run shows, in the page's words: a pass, a fail, a timeout or no exit code.
fn run_text(timed_out: bool, exit_code: Option<i32>) -> String {
    match (timed_out, exit_code) {
        (true, _) => "timed out.".into(),
        (false, Some(0)) => "exit 0 (pass).".into(),
        (false, Some(code)) => format!("exit {code} (fail)."),
        (false, None) => "no exit code.".into(),
    }
}

/// The words of the old-code heading for an expectation.
fn old_expectation_text(expect: Expect) -> &'static str {
    match expect {
        Expect::Fails => "expected: it fails",
        Expect::Same => "expected: the same output",
        Expect::Absent => "expected: the feature is missing",
        Expect::NotApplicable => "expected: none",
    }
}

/// A finding of an intent check or a review, with its question as model.rs words it and the latest disposition recorded for it.
struct FindingRow {
    id: String,
    finding: Finding,
    question: Option<&'static str>,
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
                question: question_text(
                    record["kind"].as_str().unwrap_or_default(),
                    &finding.question,
                ),
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

/// The question a finding answers, as model.rs words it: INTENT_QUESTIONS for an intent check, QUESTIONS for a review.
/// An id that is not in the list has no text.
fn question_text(record_kind: &str, question: &str) -> Option<&'static str> {
    let questions: &[(&str, &str)] = match record_kind {
        "intent-check" => &INTENT_QUESTIONS,
        "review" => &QUESTIONS,
        _ => &[],
    };
    questions
        .iter()
        .find(|(id, _)| *id == question)
        .map(|(_, text)| *text)
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

/// The summary's sentence on the issues: how many were raised and how each was settled, with a link to each.
fn findings_sentence(rows: &[FindingRow]) -> String {
    if rows.is_empty() {
        return "No issues were raised.".into();
    }
    let count = |state: &str| rows.iter().filter(|r| disposition_name(r) == state).count();
    let (issues, verb) = if rows.len() == 1 {
        ("issue", "was")
    } else {
        ("issues", "were")
    };
    let counts = format!(
        "{} {issues} {verb} raised: {} fixed, {} rejected, {} open.",
        rows.len(),
        count("fixed"),
        count("rejected"),
        count("open")
    );
    let links: Vec<String> = rows.iter().map(|r| link(&r.id)).collect();
    format!("{counts} {}", links.join(", "))
}

fn finding_html(row: &FindingRow) -> String {
    let finding = &row.finding;
    let question = match row.question {
        Some(text) => format!("{}: {}", esc(&finding.question), esc(text)),
        None => esc(&finding.question),
    };
    let state = disposition_name(row);
    let settled = match &row.disposition {
        None => "still open.".to_owned(),
        Some(d) => format!("{state}: {}", esc(&d.reason)),
    };
    format!(
        "<details id=\"{id}\" class=\"om-finding\">\n<summary><span class=\"om-id\">{id}</span> <span class=\"om-question\">{question}</span> <span class=\"om-disposition\">{state}</span></summary>\n<p><b>Issue:</b> {text}</p>\n<p><b>Evidence:</b> {evidence}</p>\n<p><b>Why it mattered:</b> {consequence}</p>\n<p><b>How it was settled:</b> {settled}</p>\n</details>\n",
        id = esc(&row.id),
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
            (
                "c1",
                "changed as intended: failed on the old code, passes now",
            ),
            ("c2", "kept: same output on the old and new code"),
            ("c3", "new: the old code did not have this"),
            (
                "c4",
                "old code did not behave as expected: expected it to fail",
            ),
            (
                "c5",
                "old code did not behave as expected: expected the same output",
            ),
            (
                "c6",
                "old code did not behave as expected: expected the feature to be missing",
            ),
            ("c7", "not run on the old code: a guard over the gate"),
            ("c8", "fails on the new code"),
            ("c9", "fails on the new code"),
            ("c10", "not run yet"),
            ("c11", "not run yet"),
        ];
        for (id, label) in expected {
            assert_eq!(label_of(&html, id), label, "row {id}");
        }
        assert!(row(&html, "c4").contains("<p><b>Old code, run 1:</b> exit 0 (pass).</p>"));
        assert!(row(&html, "c1").contains("<pre>no feature</pre>"));
        assert!(row(&html, "c10").contains("no result is recorded"));
        assert!(row(&html, "c11").contains("a different check command"));
        assert!(
            row(&html, "c11").contains("<b>On the new code:</b> exit 0 (pass).</p>\n<pre>ok</pre>"),
            "{}",
            row(&html, "c11")
        );
    }

    #[test]
    fn current_verification_names_its_record_and_adds_no_prefix() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Ok(TREE.to_owned()),
        );
        assert_eq!(
            freshness_of(&html),
            "These results are current: the last run of the checks (verification-2) used the code as it is now."
        );
        assert!(!html.contains("out of date: "));
    }

    #[test]
    fn stale_tree_prefixes_every_row_with_a_result() {
        let html = render(
            &show(false, true, labelled_context(TREE)),
            Ok("tree-later".to_owned()),
        );
        assert!(freshness_of(&html).starts_with("These results are out of date: the code changed after the last run of the checks (verification-2)."));
        assert_eq!(
            label_of(&html, "c1"),
            "out of date: changed as intended: failed on the old code, passes now"
        );
        assert_eq!(
            label_of(&html, "c2"),
            "out of date: kept: same output on the old and new code"
        );
        assert_eq!(label_of(&html, "c8"), "out of date: fails on the new code");
        assert_eq!(label_of(&html, "c10"), "not run yet");
        // c11's result is for another check: it does not count, but it is a result, so it is stale too.
        assert_eq!(label_of(&html, "c11"), "out of date: not run yet");
        assert!(!html.contains("<span class=\"om-label\">changed as intended"));
        assert!(!html.contains("<span class=\"om-label\">kept:"));
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
        assert_eq!(
            freshness_of(&html),
            "The checks have not run yet for the latest goal (intent-1)."
        );
        for id in ["c1", "c4", "c7", "c11"] {
            assert_eq!(label_of(&html, id), "not run yet", "row {id}");
        }
    }

    #[test]
    fn unreadable_latest_verification_is_not_replaced_by_an_older_one() {
        let mut context = labelled_context(TREE);
        context.push(record(
            "verification-3",
            "verification",
            3,
            json!({"intent": "intent-1", "tree": TREE, "results": null}),
        ));
        let html = render(&show(false, true, context), Ok(TREE.to_owned()));
        assert!(
            freshness_of(&html)
                .starts_with("The last run of the checks (verification-3) could not be read: "),
            "{}",
            freshness_of(&html)
        );
        for id in ["c1", "c2", "c3", "c7"] {
            assert_eq!(label_of(&html, id), "not run yet", "row {id}");
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
            "It is unknown whether these results are current: repo /nonexistent/outcome-map-test: No such file or directory"
        );
        assert_eq!(
            label_of(&html, "c1"),
            "changed as intended: failed on the old code, passes now"
        );
        assert_eq!(label_of(&html, "c8"), "fails on the new code");
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
        assert!(html.contains("<h3>Issues raised, and how each was settled</h3>"));
        assert!(html.contains("4 issues were raised: 2 fixed, 1 rejected, 1 open."));
        assert!(html.contains("<a href=\"#intent-check-1-f1\">intent-check-1-f1</a>"));
        let state = |id: &str| text_in(row(&html, id), "om-disposition").to_owned();
        assert_eq!(state("intent-check-1-f1"), "fixed");
        assert_eq!(state("intent-check-1-f2"), "rejected");
        assert_eq!(state("intent-check-1-f3"), "open");
        assert_eq!(state("review-1-f1"), "fixed");
        assert!(row(&html, "intent-check-1-f1")
            .contains("<p><b>How it was settled:</b> fixed: added the test</p>"));
        assert!(row(&html, "intent-check-1-f1").contains("<p><b>Issue:</b> text of f1</p>"));
        assert!(row(&html, "intent-check-1-f1")
            .contains("<span class=\"om-question\">use: Once this exists, what will the owner"));
        assert!(row(&html, "review-1-f1").contains(
            "<span class=\"om-question\">scope: Does the diff do only what the intent asks?"
        ));
        assert!(row(&html, "intent-check-1-f2")
            .contains("<p><b>How it was settled:</b> rejected: the goal is silent</p>"));
        assert!(row(&html, "intent-check-1-f3")
            .contains("<p><b>How it was settled:</b> still open.</p>"));
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
        assert!(!html.contains("Issues raised"));
        assert!(!html.contains("No issues were raised"));
        assert!(!html.contains("om-finding"));
        assert_eq!(label_of(&html, "c1"), "not run yet");
        assert!(html.contains(
            "<p class=\"om-summary\">1 check was approved. 1 not run yet: <a href=\"#c1\">c1</a>.</p>"
        ));
    }

    #[test]
    fn profile_b_without_findings_says_none_were_raised() {
        let context = vec![intent_record(
            "Ship the thing.",
            vec![criterion("c1", None)],
        )];
        let html = render(&show(true, false, context), Ok(TREE.to_owned()));
        assert!(html.contains("<h3>Issues raised, and how each was settled</h3>"));
        assert!(html.contains("No issues were raised."));
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
            format!("not run on the old code: {NO_OLD_CHECK}")
        );
        assert!(row(&html, "c1").contains(&format!(
            "<p><b>Not run on the old code:</b> {NO_OLD_CHECK}.</p>"
        )));
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
        assert!(html.contains("<p class=\"om-goal\"><b>The approved goal, as written:</b> Ship &lt;b&gt;&amp;&quot;&#39;</p>"));
        assert!(!html.contains("<b>&"));
    }

    /// The text a reader sees: the page's entities undone. `&amp;` goes last, so an escaped `&lt;` stays as written.
    fn unescape(html: &str) -> String {
        html.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    #[test]
    fn record_text_is_printed_verbatim() {
        let goal = "Ship <tree 4b825dc> & \"LE-212\" 'as is' for verification-7";
        let argv = json!(["sh", "-c", "test \"$X\" = '<LE-212>' & echo tree 4b825dc"]);
        let not_applicable = json!({"id": "c1", "statement": "c1 says <intent-check-1> & \"tree 4b825dc\" 'verification-7'", "check": {"argv": argv, "timeout_ms": 1000}, "old": {"expect": "none", "reason": "tree 4b825dc is <not> checked & \"LE-212\" 'guards' it"}});
        let fails = json!({"id": "c2", "statement": "c2 says LE-212 & <verification-7> \"quoted\" 'x'", "check": {"argv": ["sh", "-c", "echo verification-7 & <LE-212>"], "timeout_ms": 1000}, "old": {"expect": "fails", "reason": "the old & <LE-212> command printed 'tree 4b825dc' \"lowercase\""}});
        let results = vec![
            result(
                &not_applicable,
                Some(0),
                "verification-7 output: a < b & c > d \"e\" 'f'",
                None,
            ),
            result(
                &fails,
                Some(0),
                "new & <output> 'tree 4b825dc'",
                Some(old_record(
                    "fails",
                    vec![
                        run(Some(1), "old & <run> \"tail\" 'LE-212'"),
                        run(Some(1), "old run two <verification-7>"),
                    ],
                )),
            ),
        ];
        let reviewer = json!({"name": "fresh <reviewer> & \"LE-212\"", "kind": "agent"});
        let context = vec![
            intent_record(goal, vec![not_applicable.clone(), fails.clone()]),
            record(
                "verification-7",
                "verification",
                2,
                json!({"intent": "intent-1", "tree": TREE, "results": results}),
            ),
            record(
                "intent-check-1",
                "intent-check",
                3,
                json!({"intent": "intent-1", "reviewer": reviewer, "fresh_context": true, "answers": [], "findings": [{"id": "f1", "question": "use", "text": "Text <one> & \"two\" 'three' LE-212", "evidence": "Evidence & <tree 4b825dc> 'verification-7'", "consequence": "Consequence \"A\" & <B> intent-check-1"}]}),
            ),
            record(
                "disp-1",
                "finding-disposition",
                4,
                json!({"review": "intent-check-1", "finding": "f1", "disposition": "fixed", "reason": "Reason <LE-212> & \"tree 4b825dc\" 'fixed'"}),
            ),
            record(
                "review-1",
                "review",
                5,
                json!({"intent": "intent-1", "tree": TREE, "reviewer": reviewer, "fresh_context": true, "answers": [], "findings": [{"id": "f1", "question": "scope", "text": "Review <text> & 'LE-212'", "evidence": "Review & \"evidence\" verification-7", "consequence": "Review <consequence> & intent-check-1"}]}),
            ),
            record(
                "disp-2",
                "finding-disposition",
                6,
                json!({"review": "review-1", "finding": "f1", "disposition": "rejected", "reason": "Rejected <tree 4b825dc> & \"verification-7\" 'LE-212'"}),
            ),
        ];
        let html = render(&show(true, true, context), Ok(TREE.to_owned()));
        let text = unescape(&html);
        let recorded = [
            goal.to_owned(),
            "c1 says <intent-check-1> & \"tree 4b825dc\" 'verification-7'".to_owned(),
            "c2 says LE-212 & <verification-7> \"quoted\" 'x'".to_owned(),
            "the old & <LE-212> command printed 'tree 4b825dc' \"lowercase\"".to_owned(),
            serde_json::to_string(&argv).expect("argv serializes"),
            serde_json::to_string(&fails["check"]["argv"]).expect("argv serializes"),
            "tree 4b825dc is <not> checked & \"LE-212\" 'guards' it".to_owned(),
            "verification-7 output: a < b & c > d \"e\" 'f'".to_owned(),
            "new & <output> 'tree 4b825dc'".to_owned(),
            "old & <run> \"tail\" 'LE-212'".to_owned(),
            "old run two <verification-7>".to_owned(),
            "Text <one> & \"two\" 'three' LE-212".to_owned(),
            "Evidence & <tree 4b825dc> 'verification-7'".to_owned(),
            "Consequence \"A\" & <B> intent-check-1".to_owned(),
            "Reason <LE-212> & \"tree 4b825dc\" 'fixed'".to_owned(),
            "Review <text> & 'LE-212'".to_owned(),
            "Review & \"evidence\" verification-7".to_owned(),
            "Review <consequence> & intent-check-1".to_owned(),
            "Rejected <tree 4b825dc> & \"verification-7\" 'LE-212'".to_owned(),
        ];
        for recorded_text in &recorded {
            assert!(
                text.contains(recorded_text.as_str()),
                "not printed as recorded: {recorded_text}"
            );
        }
    }

    /// The old-code reason follows the old-code heading for fails, same and absent, with or without a recorded result.
    #[test]
    fn old_reason_follows_the_old_code_heading() {
        let criteria = vec![
            criterion(
                "c1",
                Some(json!({"expect": "fails", "reason": "fails <&> 'LE-212'"})),
            ),
            criterion(
                "c2",
                Some(json!({"expect": "same", "reason": "same & \"tree 4b825dc\""})),
            ),
            criterion(
                "c3",
                Some(json!({"expect": "absent", "reason": "absent <verification-7>"})),
            ),
            criterion("c4", Some(expect("fails"))),
        ];
        let context = vec![
            intent_record("Ship the thing.", criteria.clone()),
            verification_record(
                TREE,
                vec![result(
                    &criteria[0],
                    Some(0),
                    "ok",
                    Some(old_record(
                        "fails",
                        vec![run(Some(1), "no"), run(Some(1), "no")],
                    )),
                )],
            ),
        ];
        let html = render(&show(false, true, context), Ok(TREE.to_owned()));
        assert!(
            row(&html, "c1").contains("<p><b>On the old code (expected: it fails):</b></p>\n<p><b>Reason given:</b> fails &lt;&amp;&gt; &#39;LE-212&#39;</p>\n<p><b>Old code, run 1:</b>"),
            "{}",
            row(&html, "c1")
        );
        assert!(row(&html, "c2").ends_with("<p><b>On the old code (expected: the same output):</b></p>\n<p><b>Reason given:</b> same &amp; &quot;tree 4b825dc&quot;</p>\n</details>"), "{}", row(&html, "c2"));
        assert!(row(&html, "c3").ends_with("<p><b>On the old code (expected: the feature is missing):</b></p>\n<p><b>Reason given:</b> absent &lt;verification-7&gt;</p>\n</details>"), "{}", row(&html, "c3"));
        assert!(
            row(&html, "c4")
                .ends_with("<p><b>On the old code (expected: it fails):</b></p>\n</details>"),
            "{}",
            row(&html, "c4")
        );
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
        let header_after_main = "<main><p>x</p></main><footer><header>f</header></footer><div class=\"rail-links\"></div>";
        let error = splice(header_after_main, MARKER).expect_err("header outside main");
        assert!(error.contains("</header>"), "{error}");
        let no_rail = "<main><header>x</header><p>y</p></main>";
        let error = splice(no_rail, MARKER).expect_err("no rail");
        assert!(error.contains("rail-links"), "{error}");
    }

    /// The text of one `## ` section of the skill, from its heading to the next one.
    fn skill_section<'a>(skill: &'a str, heading: &str) -> &'a str {
        let start = skill.find(heading).expect("skill section heading");
        let rest = &skill[start + heading.len()..];
        let end = rest.find("\n## ").unwrap_or(rest.len());
        &skill[start..start + heading.len() + end]
    }

    /// The written contract (LE-212, the provider README and the skill) names every label, first line, field, anchor and
    /// check that this command and its journeys implement. c9's grep only finds the headings; this keeps the content.
    #[test]
    fn docs_state_the_whole_contract() {
        let prd = include_str!("../../../docs/PRD.md");
        let start = prd.find("### LE-212:").expect("LE-212 heading");
        let le212 = &prd[start..];
        let le212 = &le212[..le212[1..].find("\n### ").map_or(le212.len(), |end| end + 1)];
        let readme = include_str!("../README.md");
        let skill = include_str!("../skills/using-lean-change-provider/SKILL.md");
        let labels = [
            "changed as intended: failed on the old code, passes now",
            "kept: same output on the old and new code",
            "new: the old code did not have this",
            "old code did not behave as expected: expected it to fail",
            "old code did not behave as expected: expected the same output",
            "old code did not behave as expected: expected the feature to be missing",
            "not run on the old code: <reason>",
            "not run on the old code: this run has no old-code check",
            "fails on the new code",
            UNKNOWN,
            STALE,
        ];
        let first_lines = [
            "These results are current: the last run of the checks (<verification id>) used the code as it is now.",
            "These results are out of date: the code changed after the last run of the checks (<verification id>). Code fingerprint now <tree>; when checked <tree>.",
            "The checks have not run yet for the latest goal (<intent id>).",
            "The last run of the checks (<verification id>) could not be read: <error>",
            "It is unknown whether these results are current: <reason>",
        ];
        let fields = [
            "The approved goal, as written:",
            "Command that checks it:",
            "On the new code:",
            "On the old code (expected: it fails):",
            "Old code, run N:",
            "Not run on the old code:",
            "Issues raised, and how each was settled",
            "Issue:",
            "Evidence:",
            "Why it mattered:",
            "How it was settled:",
        ];
        let anchors = [
            "</header>",
            "<main",
            "</main>",
            RAIL_TAG,
            RAIL_LINK,
            "<section id=\"evidence\">",
        ];
        let checks = [
            "--only outcome-map` (criteria c1–c6)",
            "--only outcome-map-skill` (c7)",
            "--only outcome-map-fixture` (c8)",
            "--only outcome-map-browser --chromium PATH` (c11",
        ];
        for text in labels
            .iter()
            .chain(&first_lines)
            .chain(&fields)
            .chain(&anchors)
            .chain(&checks)
        {
            assert!(le212.contains(text), "LE-212 does not state {text:?}");
        }
        for text in labels.iter().chain(&first_lines) {
            assert!(readme.contains(text), "the README does not state {text:?}");
            assert!(skill.contains(text), "the skill does not state {text:?}");
        }
        for text in ["outcome-map", "--into"] {
            assert!(readme.contains(text), "the README does not state {text:?}");
        }
        let intent_step = skill_section(skill, "## The loop");
        for text in [
            "technical reader who does not know this project, the change or Loop Engine",
            "Say what the change does before how it is checked.",
            "Explain a project name the first time it appears, or leave it out.",
            "Use short sentences.",
            "A command, path or string a check depends on stays exact.",
            "Apply the no-ai-slop skill to every sentence before `intent-ready`.",
            "If that skill is not installed, tell the owner and write plainly without it.",
        ] {
            assert!(
                intent_step.contains(text),
                "the skill's intent step does not state {text:?}"
            );
        }
        let outcome_map = skill_section(skill, "## Outcome map");
        for text in [
            "Write every sentence of Accountable Review's page for that same reader",
            "Apply the no-ai-slop skill to every sentence of its prose before AR's coverage gate and lint.",
            "If that skill is not installed, tell the owner and write plainly without it.",
            "AR's own text and rules stay as they are.",
        ] {
            assert!(
                outcome_map.contains(text),
                "the skill's Outcome map step does not state {text:?}"
            );
        }
    }
}
