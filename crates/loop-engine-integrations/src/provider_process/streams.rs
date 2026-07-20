use std::io::{self, Read};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedStream {
    pub retained: Vec<u8>,
    pub original_length: usize,
    pub truncated: bool,
}

pub fn drain<R: Read>(mut reader: R, retain_limit: usize) -> io::Result<CapturedStream> {
    let mut retained = Vec::with_capacity(retain_limit.min(64 * 1024));
    let mut original_length = 0usize;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        original_length = original_length.saturating_add(read);
        let remaining = retain_limit.saturating_sub(retained.len());
        retained.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    Ok(CapturedStream {
        retained,
        original_length,
        truncated: original_length > retain_limit,
    })
}

#[cfg(test)]
mod tests {
    use super::drain;

    #[test]
    fn drains_beyond_retention_limit_without_truncating_count() {
        let bytes = vec![7; 100];
        let captured = drain(bytes.as_slice(), 10).unwrap();
        assert_eq!(captured.retained.len(), 10);
        assert_eq!(captured.original_length, 100);
        assert!(captured.truncated);
    }
}
