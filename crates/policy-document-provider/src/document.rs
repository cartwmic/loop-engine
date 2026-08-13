use crate::config::Target;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Snapshot {
    pub target_id: String,
    pub path: PathBuf,
    pub text: String,
    pub sha256: String,
}

impl Snapshot {
    pub fn read(target: &Target) -> Result<Self, String> {
        let bytes = fs::read(&target.path)
            .map_err(|e| format!("target `{}` is inaccessible: {e}", target.path))?;
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| "target is not valid UTF-8".to_owned())?;
        Ok(Self {
            target_id: target.id.clone(),
            path: PathBuf::from(&target.path),
            text,
            sha256: sha256(&bytes),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Heading {
    pub level: u8,
    pub title: String,
    pub line: usize,
}

pub fn headings(text: &str) -> Vec<Heading> {
    let mut result = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for (line, raw) in text.lines().enumerate() {
        let trimmed = raw.trim_start();
        if let Some((ch, len)) = fence {
            let close = trimmed.bytes().take_while(|b| *b == ch).count();
            if close >= len && trimmed[close..].trim().is_empty() {
                fence = None;
            }
            continue;
        }
        if let Some(opening) = opening_fence(trimmed) {
            fence = Some(opening);
            continue;
        }
        let count = trimmed.bytes().take_while(|b| *b == b'#').count();
        if !(1..=6).contains(&count)
            || !trimmed
                .as_bytes()
                .get(count)
                .is_some_and(u8::is_ascii_whitespace)
        {
            continue;
        }
        let body = trimmed[count..].trim();
        let title = body.trim_end_matches('#').trim_end().to_owned();
        if title.is_empty() {
            continue;
        }
        result.push(Heading {
            level: count as u8,
            title,
            line,
        });
    }
    result
}

pub fn heading_match(title: &str, aliases: &[String]) -> bool {
    aliases.iter().any(|a| a.eq_ignore_ascii_case(title.trim()))
}

pub fn command_in_section(text: &str, aliases: &[String]) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let hs = headings(text);
    for section in hs.iter().filter(|h| heading_match(&h.title, aliases)) {
        let mut fence: Option<(u8, usize, bool)> = None;
        for (index, line) in lines.iter().enumerate().skip(section.line + 1) {
            if let Some(h) = hs
                .iter()
                .find(|h| h.line == index && h.level <= section.level)
            {
                let _ = h;
                break;
            }
            let trimmed = line.trim_start();
            if let Some((ch, len, has_command)) = fence {
                let close = trimmed.chars().take_while(|c| *c == ch as char).count();
                if close >= len && trimmed[close..].trim().is_empty() {
                    if has_command {
                        return true;
                    }
                    fence = None;
                } else {
                    fence = Some((ch, len, has_command || is_command_line(line)));
                }
            } else if let Some((ch, len)) = opening_fence(trimmed) {
                fence = Some((ch, len, false));
            }
        }
    }
    false
}

fn opening_fence(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let ch = *bytes.first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = bytes.iter().take_while(|b| **b == ch).count();
    (len >= 3).then_some((ch, len))
}
fn is_command_line(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && !t.starts_with('#') && !t.starts_with("//") && !t.starts_with("<!--")
}

pub fn references(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut in_fence: Option<(u8, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some((ch, len)) = in_fence {
            let n = trimmed.bytes().take_while(|b| *b == ch).count();
            if n >= len && trimmed[n..].trim().is_empty() {
                in_fence = None;
            }
            continue;
        }
        if let Some(f) = opening_fence(trimmed) {
            in_fence = Some(f);
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let image = bytes[i] == b'!' && bytes.get(i + 1) == Some(&b'[');
            if (bytes[i] == b'[' && !is_escaped(bytes, i)) || (image && !is_escaped(bytes, i)) {
                let start = if image { i + 1 } else { i };
                let close = bytes[start + 1..]
                    .iter()
                    .enumerate()
                    .find(|(offset, byte)| {
                        **byte == b']' && !is_escaped(bytes, start + 1 + *offset)
                    })
                    .map(|(offset, _)| start + 1 + offset);
                if let Some(close) = close {
                    if !line[start + 1..close].contains('[') && bytes.get(close + 1) == Some(&b'(')
                    {
                        let destination_start = close + 2;
                        let angle = bytes[destination_start..]
                            .iter()
                            .position(|byte| !byte.is_ascii_whitespace())
                            .is_some_and(|offset| bytes[destination_start + offset] == b'<');
                        let end = if angle {
                            let close_angle = bytes[destination_start + 1..]
                                .iter()
                                .enumerate()
                                .find(|(offset, byte)| {
                                    **byte == b'>'
                                        && !is_escaped(bytes, destination_start + 1 + *offset)
                                })
                                .map(|(offset, _)| destination_start + 1 + offset);
                            close_angle.and_then(|angle_end| {
                                bytes[angle_end + 1..]
                                    .iter()
                                    .enumerate()
                                    .find(|(offset, byte)| {
                                        **byte == b')'
                                            && !is_escaped(bytes, angle_end + 1 + *offset)
                                    })
                                    .map(|(offset, _)| angle_end + 1 + offset - destination_start)
                            })
                        } else {
                            bytes[destination_start..]
                                .iter()
                                .enumerate()
                                .find(|(offset, byte)| {
                                    **byte == b')'
                                        && !is_escaped(bytes, destination_start + *offset)
                                })
                                .map(|(offset, _)| offset)
                        };
                        if let Some(end) = end {
                            let raw = &line[destination_start..destination_start + end];
                            // Nested/escaped destination syntax is outside frozen grammar.
                            let angle_destination = raw.trim_start().starts_with('<');
                            if (angle_destination || !raw.contains('(')) && !raw.contains('\\') {
                                result.push(extract_destination(raw));
                            }
                            i = close + 3 + end;
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }
        if let Some(import) = trimmed.strip_prefix('@') {
            result.push(extract_destination(import));
        }
    }
    result
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut slashes = 0;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        slashes += 1;
        cursor -= 1;
    }
    slashes % 2 == 1
}

fn extract_destination(value: &str) -> String {
    let value = value.trim_start();
    if let Some(angle) = value.strip_prefix('<') {
        return angle
            .split_once('>')
            .map_or("", |(path, _)| path)
            .to_owned();
    }
    value.split_whitespace().next().unwrap_or("").to_owned()
}

pub fn resolve_reference(base: &Path, reference: &str) -> Result<PathBuf, String> {
    let mut reference = reference.trim().to_owned();
    if reference.is_empty() {
        return Err("empty reference".into());
    }
    if reference.starts_with('<') || reference.ends_with('>') {
        reference = reference.trim_matches('<').trim_matches('>').to_owned();
    }
    let original_lower = reference.to_ascii_lowercase();
    if ["http:", "https:", "mailto:", "data:", "#", "//"]
        .iter()
        .any(|prefix| original_lower.starts_with(prefix))
    {
        return Ok(PathBuf::new());
    }
    if let Some(index) = reference.find(['?', '#']) {
        reference.truncate(index);
    }
    if reference.is_empty() {
        return Err("empty reference".into());
    }
    if reference.contains('%') {
        return Err(format!("unsupported percent-encoding in `{reference}`"));
    }
    let mut out = PathBuf::from(base);
    for component in Path::new(&reference).components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(s) => out.push(s),
            std::path::Component::ParentDir => {
                if !out.pop() || out == base.parent().unwrap_or(base) {
                    return Err(format!("reference escapes target directory: `{reference}`"));
                }
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!("absolute reference is unsupported: `{reference}`"))
            }
        }
    }
    if out.exists() {
        Ok(out)
    } else {
        Err(format!("reference does not resolve: `{reference}`"))
    }
}

fn sha256(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut x) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = x
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            x = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (v, n) in [a, b, c, d, e, f, g, x].iter().zip(h.iter_mut()) {
            *n = n.wrapping_add(*v);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn directory() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "policy-document-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn headings_follow_atx_and_fence_grammar() {
        let text = "```md\n# Fake\n## Purpose\n```\n# Real ###\n#\n##Purpose\n~~~\n## Also fake\n~~~~\n## Usage\n";
        let found = headings(text);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].level, found[0].title.as_str()), (1, "Real"));
        assert_eq!((found[1].level, found[1].title.as_str()), (2, "Usage"));
        assert!(!headings("#   ###\n")
            .iter()
            .any(|heading| heading.level == 1));
    }

    #[test]
    fn command_sections_require_non_comment_fenced_content_and_honor_boundaries() {
        let aliases = vec!["Setup".to_owned()];
        assert!(command_in_section(
            "## SETUP\n```sh\n cargo build\n```\n",
            &aliases
        ));
        assert!(command_in_section(
            "## Setup\n~~~text\nrun tool\n~~~~\n",
            &aliases
        ));
        assert!(!command_in_section(
            "## Setup\n```sh\n# no\n// no\n<!-- no -->\n```\n",
            &aliases
        ));
        assert!(!command_in_section(
            "## Setup\n```sh\n## Fake boundary\n```\n## Next\n```sh\nrun\n```\n",
            &aliases
        ));
        assert!(!command_in_section(
            "```md\n## Setup\n```sh\nrun\n```\n```\n",
            &aliases
        ));
    }

    #[test]
    fn fence_character_length_and_comment_prefixes_are_exact() {
        let aliases = vec!["Setup".to_owned()];
        assert!(command_in_section(
            "## Setup\n````sh\nrun\n`````\n",
            &aliases
        ));
        assert!(!command_in_section(
            "## Setup\n````sh\nrun\n~~~\n",
            &aliases
        ));
        assert!(command_in_section(
            "## Setup\n```sh\n#comment\n//comment\n<!--comment\n echo ok\n```\n",
            &aliases
        ));
        assert!(!command_in_section(
            "## Setup\n```sh\n# comment only\n```\n## Next\n````sh\nrun\n```\n",
            &aliases
        ));
    }

    #[test]
    fn reference_tokenizer_handles_supported_skipped_and_fenced_forms() {
        let found = references("[doc](guide.md#top) ![pic](<img file.png> \"title\")\n@ <AGENTS.md> ignored\n[web](https://example.test)\n```md\n[ignored](missing.md)\n```\n[label]: ref.md\n<a href=\"x\">x</a>\n\\[escaped](no.md)\n");
        assert_eq!(
            found,
            vec![
                "guide.md#top",
                "img file.png",
                "AGENTS.md",
                "https://example.test"
            ]
        );
        assert_eq!(
            references("[]( )\n[outer [inner]](ignored.md)\n![x](data:image/png,x)\n[x](#part)\n[x](mailto:a@b)\n@\n"),
            vec!["", "data:image/png,x", "#part", "mailto:a@b", ""]
        );
        assert_eq!(
            references(r"\[escaped](ignored.md) [x\](ignored.md) [x](nested(path).md)"),
            Vec::<String>::new()
        );
        assert_eq!(
            references("[doc](<guide(v2).md>) ![img](<asset(v2).png> \"title\")"),
            vec!["guide(v2).md", "asset(v2).png"]
        );
    }

    #[test]
    fn references_resolve_lexically_and_report_unsupported_paths() {
        let root = directory();
        fs::write(root.join("exists.md"), "ok").unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        assert!(resolve_reference(&root, "docs/../exists.md?view=1#top").is_ok());
        assert!(resolve_reference(&root, "https://example.test/x")
            .unwrap()
            .as_os_str()
            .is_empty());
        assert!(resolve_reference(&root, "//example.test/x")
            .unwrap()
            .as_os_str()
            .is_empty());
        assert!(resolve_reference(&root, "exists.md?view=100%25#top").is_ok());
        assert!(resolve_reference(&root, "missing.md")
            .unwrap_err()
            .contains("does not resolve"));
        assert!(resolve_reference(&root, "../outside.md")
            .unwrap_err()
            .contains("escapes"));
        assert!(resolve_reference(&root, "space%20name.md")
            .unwrap_err()
            .contains("percent"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_is_exact_utf8_and_sha256() {
        let root = directory();
        let path = root.join("target.md");
        fs::write(&path, b"abc").unwrap();
        let target = Target {
            id: "target".into(),
            path: path.display().to_string(),
        };
        let snapshot = Snapshot::read(&target).unwrap();
        assert_eq!(snapshot.text, "abc");
        assert_eq!(
            snapshot.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::write(&path, [0xff]).unwrap();
        assert_eq!(
            Snapshot::read(&target).unwrap_err(),
            "target is not valid UTF-8"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
