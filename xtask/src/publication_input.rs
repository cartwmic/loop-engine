//! Report-neutral parsing and lossless evidence for Git publication inputs.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputEvidence {
    pub encoding: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTuple {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_ref: String,
    pub remote_sha: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCode {
    MalformedUpdateInput,
    InvalidUpdateShape,
    MultipleContentTips,
    MalformedCiEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedUpdateDisposition {
    Content(UpdateTuple),
    DeletionOnly,
    Rejected(RejectionCode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUpdates {
    pub input_evidence: InputEvidence,
    pub updates: Vec<UpdateTuple>,
    pub disposition: ParsedUpdateDisposition,
}

/// Parse complete pre-push stdin into one canonical aggregate disposition.
///
/// Canonical update evidence is stably tuple-sorted. Exact duplicate content
/// lines remain distinct occurrences and therefore reject as multiple tips.
pub fn parse_updates(input: &[u8]) -> ParsedUpdates {
    let input_evidence = input_evidence(input);
    let Ok(text) = std::str::from_utf8(input) else {
        return ParsedUpdates {
            input_evidence,
            updates: Vec::new(),
            disposition: ParsedUpdateDisposition::Rejected(RejectionCode::MalformedUpdateInput),
        };
    };
    if text.is_empty() {
        return ParsedUpdates {
            input_evidence,
            updates: Vec::new(),
            disposition: ParsedUpdateDisposition::DeletionOnly,
        };
    }

    let mut updates = Vec::new();
    let mut lines = text.split('\n').collect::<Vec<_>>();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    for line in lines {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 4 {
            return ParsedUpdates {
                input_evidence,
                updates: Vec::new(),
                disposition: ParsedUpdateDisposition::Rejected(RejectionCode::MalformedUpdateInput),
            };
        }
        updates.push(UpdateTuple {
            local_ref: fields[0].to_owned(),
            local_sha: fields[1].to_owned(),
            remote_ref: fields[2].to_owned(),
            remote_sha: fields[3].to_owned(),
        });
    }
    updates.sort_by(|left, right| tuple_key(left).cmp(&tuple_key(right)));

    if !updates_have_valid_shape(&updates) {
        return ParsedUpdates {
            input_evidence,
            updates,
            disposition: ParsedUpdateDisposition::Rejected(RejectionCode::InvalidUpdateShape),
        };
    }
    let content = updates
        .iter()
        .filter(|update| !is_zero_oid(&update.local_sha))
        .collect::<Vec<_>>();
    let disposition = match content.as_slice() {
        [] => ParsedUpdateDisposition::DeletionOnly,
        [update] => ParsedUpdateDisposition::Content((*update).clone()),
        _ => ParsedUpdateDisposition::Rejected(RejectionCode::MultipleContentTips),
    };
    ParsedUpdates {
        input_evidence,
        updates,
        disposition,
    }
}

/// Decode lossless input evidence. Base64 accepts only canonical padded RFC 4648
/// standard-alphabet text.
pub fn decode_input_evidence(evidence: &InputEvidence) -> Result<Vec<u8>> {
    match evidence.encoding.as_str() {
        "utf-8" => Ok(evidence.data.as_bytes().to_vec()),
        "base64" => decode_base64(&evidence.data).map_err(anyhow::Error::msg),
        _ => bail!("input_evidence encoding must be utf-8 or base64"),
    }
}

fn input_evidence(input: &[u8]) -> InputEvidence {
    match std::str::from_utf8(input) {
        Ok(text) => InputEvidence {
            encoding: "utf-8".to_owned(),
            data: text.to_owned(),
        },
        Err(_) => InputEvidence {
            encoding: "base64".to_owned(),
            data: encode_base64(input),
        },
    }
}

fn updates_have_valid_shape(updates: &[UpdateTuple]) -> bool {
    let mut hash_length = None;
    updates.iter().all(|update| {
        for oid in [&update.local_sha, &update.remote_sha] {
            if !valid_oid(oid) {
                return false;
            }
            match hash_length {
                Some(length) if length != oid.len() => return false,
                None => hash_length = Some(oid.len()),
                _ => {}
            }
        }
        if !valid_ref(&update.remote_ref) {
            return false;
        }
        if is_zero_oid(&update.local_sha) {
            update.local_ref == "(delete)" && !is_zero_oid(&update.remote_sha)
        } else {
            valid_ref(&update.local_ref) && update.local_ref != "(delete)"
        }
    })
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_zero_oid(value: &str) -> bool {
    valid_oid(value) && value.bytes().all(|byte| byte == b'0')
}

fn valid_ref(value: &str) -> bool {
    if !value.starts_with("refs/")
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
    {
        return false;
    }
    value.split('/').all(|component| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.starts_with('.')
            && !component.ends_with(".lock")
            && !component.bytes().any(|byte| {
                byte <= b' '
                    || byte == 0x7f
                    || matches!(byte, b'~' | b'^' | b':' | b'?' | b'*' | b'[' | b'\\')
            })
    })
}

fn tuple_key(update: &UpdateTuple) -> (&str, &str, &str, &str) {
    (
        &update.local_ref,
        &update.local_sha,
        &update.remote_ref,
        &update.remote_sha,
    )
}

fn decode_base64(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(4) {
        return Err("base64 stream length is not a multiple of four".to_owned());
    }
    let mut output = Vec::with_capacity(text.len() / 4 * 3);
    for (index, chunk) in text.as_bytes().chunks_exact(4).enumerate() {
        let last = index + 1 == text.len() / 4;
        let padding = match (chunk[2], chunk[3]) {
            (b'=', b'=') => 2,
            (_, b'=') => 1,
            (_, _) => 0,
        };
        if padding > 0 && !last {
            return Err("base64 stream padding appears before final quartet".to_owned());
        }
        let a = base64_value(chunk[0])?;
        let b = base64_value(chunk[1])?;
        let c = if chunk[2] == b'=' {
            0
        } else {
            base64_value(chunk[2])?
        };
        let d = if chunk[3] == b'=' {
            0
        } else {
            base64_value(chunk[3])?
        };
        if chunk[0] == b'=' || chunk[1] == b'=' || (chunk[2] == b'=' && chunk[3] != b'=') {
            return Err("base64 stream has invalid padding".to_owned());
        }
        output.push((a << 2) | (b >> 4));
        if padding < 2 {
            output.push((b << 4) | (c >> 2));
        }
        if padding == 0 {
            output.push((c << 6) | d);
        }
    }
    if encode_base64(&output) != text {
        return Err("base64 stream is not canonical".to_owned());
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Result<u8, String> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("base64 stream contains an invalid character".to_owned()),
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(TABLE[usize::from(first >> 2)]));
        encoded.push(char::from(
            TABLE[usize::from(((first & 0b0000_0011) << 4) | (second >> 4))],
        ));
        if chunk.len() > 1 {
            encoded.push(char::from(
                TABLE[usize::from(((second & 0b0000_1111) << 2) | (third >> 6))],
            ));
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(char::from(TABLE[usize::from(third & 0b0011_1111)]));
        } else {
            encoded.push('=');
        }
    }
    encoded
}
