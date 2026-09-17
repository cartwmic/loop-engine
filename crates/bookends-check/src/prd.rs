use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CoverageClass {
    E2eJourney,
    Contract,
}

impl CoverageClass {
    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::E2eJourney => "e2e/journey",
            Self::Contract => "contract",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordKind {
    Live { classes: Vec<CoverageClass> },
    Tombstone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    pub id: String,
    pub title: String,
    pub kind: RecordKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Prd {
    pub records: Vec<Record>,
}

impl Prd {
    pub(crate) fn live_ids(&self) -> Vec<String> {
        self.records
            .iter()
            .filter(|r| matches!(r.kind, RecordKind::Live { .. }))
            .map(|r| r.id.clone())
            .collect()
    }

    pub(crate) fn live_id_set(&self) -> BTreeSet<String> {
        self.live_ids().into_iter().collect()
    }

    pub(crate) fn tombstone_ids(&self) -> BTreeSet<String> {
        self.records
            .iter()
            .filter(|r| matches!(r.kind, RecordKind::Tombstone))
            .map(|r| r.id.clone())
            .collect()
    }
}

/// Parse a proposed PRD without consulting repository configuration,
/// citations, CI, or continuity.  This is the candidate-only validation
/// surface used before a human accepts a proposal into the living PRD.
///
/// A candidate must contain at least one live record or retained tombstone;
/// no repository-level claim is made by this function.
pub fn validate_candidate(text: &str) -> Result<Vec<String>, Vec<String>> {
    Ok(parse_candidate(text)?.live_ids())
}

/// Return every requirement identity parsed from a candidate, including a
/// retained tombstone.  This is the same parser-only validation as
/// [`validate_candidate`], but callers that bind a provisional identity need
/// the candidate's complete parsed record set rather than only its live IDs.
pub fn candidate_ids(text: &str) -> Result<Vec<String>, Vec<String>> {
    Ok(parse_candidate(text)?
        .records
        .into_iter()
        .map(|record| record.id)
        .collect())
}

fn parse_candidate(text: &str) -> Result<Prd, Vec<String>> {
    let prd = parse_prd(text)?;
    if prd.records.is_empty() {
        return Err(vec![
            "candidate contains no live or tombstone records".to_owned()
        ]);
    }
    Ok(prd)
}

pub(crate) fn parse_prd(text: &str) -> Result<Prd, Vec<String>> {
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    let headings = h3_indices(&lines);
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let mut seen_ids = BTreeSet::new();

    for (heading_idx, &line_idx) in headings.iter().enumerate() {
        let remainder = &lines[line_idx]["### ".len()..];
        if !remainder.starts_with("LE-") {
            continue;
        }
        let end = headings
            .get(heading_idx + 1)
            .copied()
            .unwrap_or(lines.len());
        match parse_id_heading(remainder) {
            Ok((id, title)) => {
                if !seen_ids.insert(id.clone()) {
                    errors.push(format!(
                        "duplicate ID {id} across live records and retained tombstones"
                    ));
                }
                match parse_record_body(&lines[line_idx + 1..end]) {
                    Ok(kind) => records.push(Record { id, title, kind }),
                    Err(err) => errors.push(format!("{id}: {err}")),
                }
            }
            Err(err) => errors.push(format!("malformed ID-record heading: {err}")),
        }
    }

    if errors.is_empty() {
        Ok(Prd { records })
    } else {
        Err(errors)
    }
}

fn h3_indices(lines: &[&str]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_h3(line))
        .map(|(i, _)| i)
        .collect()
}

fn is_h3(line: &str) -> bool {
    line.starts_with("### ")
}

fn parse_id_heading(remainder: &str) -> Result<(String, String), String> {
    let after_le = remainder.strip_prefix("LE-").unwrap_or(remainder);
    if after_le.is_empty()
        || !after_le
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() && c != '0')
    {
        return Err(format!("### {remainder}"));
    }
    let digits: String = after_le
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.starts_with('0') {
        return Err(format!("### {remainder}"));
    }
    let rest = &after_le[digits.len()..];
    let Some(title) = rest.strip_prefix(": ") else {
        return Err(format!("### {remainder}"));
    };
    if title.trim().is_empty() {
        return Err(format!("### {remainder}"));
    }
    Ok((format!("LE-{digits}"), title.to_string()))
}

fn parse_record_body(body: &[&str]) -> Result<RecordKind, String> {
    let mut status: Option<&str> = None;
    let mut coverage: Option<&str> = None;
    for line in body {
        let line = line.trim_end();
        if line.starts_with("- Status:") {
            if line != "- Status: live" && line != "- Status: tombstone" {
                return Err(format!("invalid Status line {line}"));
            }
            if status.is_some() {
                return Err("duplicate Status".into());
            }
            status = Some(line);
            continue;
        }
        if line.starts_with("- Coverage:") {
            if coverage.is_some() {
                return Err("duplicate Coverage".into());
            }
            if status.is_none() {
                return Err("Coverage must follow Status".into());
            }
            coverage = Some(line);
            continue;
        }
    }
    match (status, coverage) {
        (Some("- Status: live"), Some(cov)) => {
            let classes = parse_coverage(cov)?;
            Ok(RecordKind::Live { classes })
        }
        (Some("- Status: live"), None) => Err("live record without Coverage".into()),
        (Some("- Status: tombstone"), None) => Ok(RecordKind::Tombstone),
        (Some("- Status: tombstone"), Some(_)) => {
            Err("tombstone record must not have a Coverage line".into())
        }
        (Some(other), _) => Err(format!("invalid Status line {other}")),
        (None, _) => Err("missing Status".into()),
    }
}

fn parse_coverage(line: &str) -> Result<Vec<CoverageClass>, String> {
    match line {
        "- Coverage: e2e/journey" => Ok(vec![CoverageClass::E2eJourney]),
        "- Coverage: e2e/journey, contract" => {
            Ok(vec![CoverageClass::E2eJourney, CoverageClass::Contract])
        }
        other => {
            if let Some(rest) = other.strip_prefix("- Coverage: ") {
                if rest.split(',').any(|part| {
                    let token = part.trim();
                    token != "e2e/journey" && token != "contract" && !token.is_empty()
                }) {
                    return Err(format!("unknown coverage class in {other}"));
                }
            }
            Err(format!("invalid Coverage line {other}"))
        }
    }
}

/// Citation token `bookends` + `:LE-<n>` with `<n>` = `[1-9][0-9]*`.
///
/// A citation is a directive comment, not an arbitrary substring.  Keeping
/// this lexical boundary prevents a fixture string or a documentation example
/// from becoming proof while retaining the existing comment directive syntax
/// used by Python and Rust proof files.
pub(crate) fn scan_citation_tokens(text: &str) -> Result<Vec<String>, Vec<String>> {
    const NEEDLE: &str = concat!("bookends", ":LE-");
    const NON_CANONICAL_NEEDLE: &str = concat!("@", "spec", ":");
    let comments = directive_comments(text);
    let mut ids = Vec::new();
    let mut errors = Vec::new();

    for comment in comments {
        let mut search_from = 0;
        while let Some(rel) = comment[search_from..].find(NEEDLE) {
            let after = search_from + rel + NEEDLE.len();
            let rest = &comment[after..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() || digits.starts_with('0') {
                errors.push(format!(
                    "malformed citation token {NEEDLE}{}",
                    rest.chars().take(8).collect::<String>()
                ));
                let advance = rest.chars().next().map(char::len_utf8).unwrap_or(0);
                search_from = after + advance;
                continue;
            }
            ids.push(format!("LE-{digits}"));
            search_from = after + digits.len();
        }

        // Compass's alternate citation spelling is explicitly not part of v1.
        // Treat it as malformed in a real directive comment rather than
        // silently ignoring it when another canonical citation is present.
        let mut search_from = 0;
        while let Some(rel) = comment[search_from..].find(NON_CANONICAL_NEEDLE) {
            let start = search_from + rel;
            let after = start + NON_CANONICAL_NEEDLE.len();
            let suffix: String = comment[after..].chars().take(24).collect();
            errors.push(format!(
                "malformed non-canonical citation token {NON_CANONICAL_NEEDLE}{suffix}"
            ));
            let advance = comment[start..]
                .chars()
                .next()
                .map(char::len_utf8)
                .unwrap_or(0);
            search_from = start + advance;
        }
    }

    if errors.is_empty() {
        Ok(ids)
    } else {
        Err(errors)
    }
}

pub(crate) fn has_skip_marker(text: &str) -> bool {
    directive_comments(text)
        .into_iter()
        .any(|comment| comment.contains("bookends:skip"))
}

/// Return ordinary line/block comment bodies while ignoring quoted strings
/// and Rust documentation comments.  This is intentionally a small lexical
/// scanner, not a language parser or a test classifier.
fn directive_comments(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut comments = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            let doc = matches!(bytes.get(index + 2), Some(b'/') | Some(b'!'));
            let start = index + 2;
            let end = skip_line(bytes, start);
            if !doc {
                comments.push(&text[start..end]);
            }
            index = end;
            continue;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            let doc = matches!(bytes.get(index + 2), Some(b'*') | Some(b'!'));
            let content_start = index + 2;
            let end = skip_block(bytes, content_start);
            let content_end = end.saturating_sub(2).max(content_start).min(bytes.len());
            if !doc {
                comments.push(&text[content_start..content_end]);
            }
            index = end;
            continue;
        }
        if bytes[index] == b'#'
            && bytes.get(index + 1) != Some(&b'[')
            && !(bytes.get(index + 1) == Some(&b'!') && bytes.get(index + 2) == Some(&b'['))
        {
            let start = index + 1;
            let end = skip_line(bytes, start);
            comments.push(&text[start..end]);
            index = end;
            continue;
        }
        if let Some(end) = raw_string_end(bytes, index) {
            index = end;
            continue;
        }
        if bytes[index] == b'"' {
            index = quoted_end(bytes, index, b'"');
            continue;
        }
        if bytes[index] == b'\'' {
            index = single_quoted_end(bytes, index);
            continue;
        }
        index += 1;
    }
    comments
}

fn skip_line(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block(bytes: &[u8], mut index: usize) -> usize {
    let mut depth = 1;
    while index < bytes.len() && depth > 0 {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            depth += 1;
            index += 2;
        } else if bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/') {
            depth -= 1;
            index += 2;
        } else {
            index += 1;
        }
    }
    index
}

fn raw_string_end(bytes: &[u8], index: usize) -> Option<usize> {
    let mut marker = index;
    if bytes.get(marker) == Some(&b'b') {
        marker += 1;
    }
    if bytes.get(marker) != Some(&b'r') {
        return None;
    }
    marker += 1;
    let mut hashes = 0;
    while bytes.get(marker) == Some(&b'#') {
        hashes += 1;
        marker += 1;
    }
    if bytes.get(marker) != Some(&b'"') {
        return None;
    }
    let mut cursor = marker + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes
                .get(cursor + 1..cursor + 1 + hashes)
                .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
        {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Some(bytes.len())
}

fn quoted_end(bytes: &[u8], index: usize, quote: u8) -> usize {
    let mut cursor = index + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            cursor = cursor.saturating_add(2);
        } else if bytes[cursor] == quote {
            return cursor + 1;
        } else {
            cursor += 1;
        }
    }
    bytes.len()
}

fn single_quoted_end(bytes: &[u8], index: usize) -> usize {
    if bytes.get(index..index + 3) == Some(b"'''") {
        let mut cursor = index + 3;
        while cursor + 2 < bytes.len() {
            if bytes.get(cursor..cursor + 3) == Some(b"'''") {
                return cursor + 3;
            }
            cursor += 1;
        }
        return bytes.len();
    }
    let end = quoted_end(bytes, index, b'\'');
    let line_end = skip_line(bytes, index);
    if end <= line_end && bytes.get(end.saturating_sub(1)) == Some(&b'\'') {
        end
    } else {
        // An apostrophe followed by an identifier is normally a Rust
        // lifetime (`'a`/`'static`), not an unterminated string.
        index + 1
    }
}

#[cfg(test)]
fn extract_fenced_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        rest = &rest[start + 3..];
        if let Some(nl) = rest.find('\n') {
            rest = &rest[nl + 1..];
        }
        match rest.find("```") {
            Some(end) => {
                blocks.push(rest[..end].to_string());
                rest = &rest[end + 3..];
            }
            None => break,
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_record_parses() {
        let prd = parse_prd(
            "### LE-1: Preserve control state\n- Status: live\n- Coverage: e2e/journey\n",
        )
        .unwrap();
        assert_eq!(prd.live_ids(), vec!["LE-1".to_string()]);
    }

    #[test]
    fn candidate_ids_expose_all_parser_records_without_changing_live_projection() {
        let text = "### LE-7: Proposed\n- Status: tombstone\n\n### LE-8: Live\n- Status: live\n- Coverage: e2e/journey\n";
        assert_eq!(
            candidate_ids(text).unwrap(),
            vec!["LE-7".to_owned(), "LE-8".to_owned()]
        );
        assert_eq!(validate_candidate(text).unwrap(), vec!["LE-8".to_owned()]);
    }

    #[test]
    fn human_prose_heading_is_not_malformed() {
        let prd = parse_prd("### 3.1 Goals\n\nLoop Engine v2 must:\n").unwrap();
        assert!(prd.records.is_empty());
    }

    #[test]
    fn malformed_id_heading_fails() {
        let err =
            parse_prd("### LE-0: nope\n- Status: live\n- Coverage: e2e/journey\n").unwrap_err();
        assert!(err.iter().any(|e| e.contains("malformed")), "{err:?}");
    }

    #[test]
    fn duplicate_ids_fail() {
        let err = parse_prd(
            "### LE-1: A\n- Status: live\n- Coverage: e2e/journey\n\n\
             ### LE-1: B\n- Status: tombstone\n",
        )
        .unwrap_err();
        assert!(err.iter().any(|e| e.contains("duplicate")), "{err:?}");
    }

    #[test]
    fn tombstone_with_coverage_fails() {
        let err =
            parse_prd("### LE-1: A\n- Status: tombstone\n- Coverage: e2e/journey\n").unwrap_err();
        assert!(err.iter().any(|e| e.contains("tombstone")), "{err:?}");
    }

    #[test]
    fn unsupported_status_value_fails() {
        let err =
            parse_prd("### LE-1: A\n- Status: draft\n- Status: live\n- Coverage: e2e/journey\n")
                .unwrap_err();
        assert!(err.iter().any(|e| e.contains("invalid Status")), "{err:?}");
    }

    #[test]
    fn duplicate_live_status_fails() {
        let err =
            parse_prd("### LE-1: A\n- Status: live\n- Status: live\n- Coverage: e2e/journey\n")
                .unwrap_err();
        assert!(err.iter().any(|e| e.contains("duplicate")), "{err:?}");
    }

    #[test]
    fn coverage_before_status_fails() {
        let err = parse_prd("### LE-1: A\n- Coverage: e2e/journey\n- Status: live\n").unwrap_err();
        assert!(
            err.iter()
                .any(|e| e.contains("Coverage") || e.contains("Status")),
            "{err:?}"
        );
    }

    #[test]
    fn unsupported_status_alone_fails() {
        let err = parse_prd("### LE-1: A\n- Status: draft\n- Coverage: e2e/journey\n").unwrap_err();
        assert!(err.iter().any(|e| e.contains("invalid Status")), "{err:?}");
    }

    #[test]
    fn citation_le10_is_not_le1() {
        let ids = scan_citation_tokens("// bookends:LE-10").unwrap();
        assert_eq!(ids, vec!["LE-10".to_string()]);
    }

    #[test]
    fn malformed_citation_le0() {
        let err = scan_citation_tokens("// bookends:LE-0").unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn malformed_unicode_citation_does_not_panic() {
        let text = "// bookends:LE-😀";
        assert!(scan_citation_tokens(text).is_err());
    }

    #[test]
    fn noncanonical_at_spec_citation_is_malformed() {
        let noncanonical = format!("// {}LE-1\n// bookends:LE-1", ["@", "spec:"].concat());
        let err = scan_citation_tokens(&noncanonical).unwrap_err();
        assert!(
            err.iter().any(|error| error.contains("non-canonical")),
            "{err:?}"
        );
    }

    #[test]
    fn noncanonical_marker_is_not_literal_in_checker_source() {
        let source = include_str!("prd.rs");
        let marker = ["@", "spec", ":LE-"].concat();
        assert!(
            !source.contains(&marker),
            "checker source must construct the rejected marker from fragments"
        );
    }

    #[test]
    fn citations_ignore_strings_and_documentation_comments() {
        let text = r#"
            const EXAMPLE: &str = "bookends:LE-1";
            /// bookends:LE-2
            /** bookends:LE-3 */
            let single = 'bookends:LE-4';
            // bookends:LE-5
            /* bookends:LE-6 */
        "#;
        assert_eq!(
            scan_citation_tokens(text).unwrap(),
            vec!["LE-5".to_owned(), "LE-6".to_owned()]
        );
    }

    #[test]
    fn skip_marker_is_only_a_comment_directive() {
        let text = r#"
            let value = "bookends:skip";
            // bookends:LE-1
        "#;
        assert!(!has_skip_marker(text));
        assert_eq!(scan_citation_tokens(text).unwrap(), vec!["LE-1"]);
        assert!(has_skip_marker("// bookends:skip\n// bookends:LE-1"));
    }

    #[test]
    fn a_rust_lifetime_does_not_hide_a_following_comment() {
        let text = "let value: &'static str = \"value\"; // bookends:LE-1";
        assert_eq!(scan_citation_tokens(text).unwrap(), vec!["LE-1"]);
    }

    #[test]
    fn raw_strings_do_not_hide_or_create_directives() {
        let text = r##"
            let value = r#"// bookends:LE-1"#;
            // bookends:LE-2
        "##;
        assert_eq!(scan_citation_tokens(text).unwrap(), vec!["LE-2"]);
    }

    #[test]
    fn mapping_excerpts_parse() {
        let mapping = include_str!("../schema/pattern-mapping.md");
        let blocks = extract_fenced_blocks(mapping);
        assert!(
            blocks.len() >= 9,
            "expected representative excerpts, got {}",
            blocks.len()
        );
        let mut live_records = 0;
        let mut tombstones = 0;
        for (i, block) in blocks.iter().enumerate() {
            let prd = parse_prd(block).unwrap_or_else(|err| {
                panic!("mapping excerpt {i} failed to parse: {err:?}\n{block}")
            });
            live_records += prd
                .records
                .iter()
                .filter(|r| matches!(r.kind, RecordKind::Live { .. }))
                .count();
            tombstones += prd
                .records
                .iter()
                .filter(|r| matches!(r.kind, RecordKind::Tombstone))
                .count();
        }
        assert!(
            live_records >= 3,
            "expected v1 live transcriptions from each corpus, got {live_records}"
        );
        assert!(
            tombstones >= 2,
            "expected tombstone transcriptions, got {tombstones}"
        );
    }

    #[test]
    fn title_is_record_field() {
        let a =
            parse_prd("### LE-1: Old title\n- Status: live\n- Coverage: e2e/journey\n").unwrap();
        let b =
            parse_prd("### LE-1: New title\n- Status: live\n- Coverage: e2e/journey\n").unwrap();
        assert_eq!(a.records[0].id, b.records[0].id);
        assert_ne!(a.records[0].title, b.records[0].title);
    }
}
