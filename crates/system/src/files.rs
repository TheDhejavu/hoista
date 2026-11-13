//! Reading files from disk safely: never more than a set size, and config
//! files written as JSON with comments and trailing commas (Wrangler's JSONC).

use std::path::Path;

/// Why a file could not be read or parsed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FileError {
    /// The file could not be opened or read.
    #[error("Could not read the file.")]
    Unreadable,
    /// The file is larger than the limit the caller set.
    #[error("The file is larger than allowed.")]
    TooLarge,
    /// The text is not JSON, even allowing comments and trailing commas.
    #[error("The file is not valid JSON with comments.")]
    InvalidJsonc,
}

/// Reads at most `limit` bytes of `path`.
///
/// # Errors
/// Returns [`FileError::TooLarge`] when the file is longer than `limit`, and
/// [`FileError::Unreadable`] when it can't be opened or read.
pub fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, FileError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| FileError::Unreadable)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| FileError::Unreadable)?;
    if bytes.len() as u64 > limit {
        return Err(FileError::TooLarge);
    }
    Ok(bytes)
}

/// Parses JSON that may contain `//` and `/* */` comments and trailing commas.
/// Callers limit the input size.
///
/// # Errors
/// Returns [`FileError::InvalidJsonc`] when the text isn't valid JSON once
/// comments and trailing commas are removed.
pub fn parse_jsonc(input: &[u8]) -> Result<serde_json::Value, FileError> {
    let mut bytes = input.to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            match bytes[i] {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'"' => quoted = false,
                _ => {}
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
                bytes[i] = b' ';
                i += 1;
            }
            continue;
        } else if bytes[i..].starts_with(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            loop {
                if i + 1 >= bytes.len() {
                    return Err(FileError::InvalidJsonc);
                }
                if bytes[i..].starts_with(b"*/") {
                    bytes[i] = b' ';
                    bytes[i + 1] = b' ';
                    i += 2;
                    break;
                }
                if !matches!(bytes[i], b'\n' | b'\r') {
                    bytes[i] = b' ';
                }
                i += 1;
            }
            continue;
        }
        i += 1;
    }
    if quoted {
        return Err(FileError::InvalidJsonc);
    }
    i = 0;
    let mut previous = None;
    while i < bytes.len() {
        let byte = bytes[i];
        if quoted {
            if byte == b'\\' {
                i += 2;
                continue;
            }
            if byte == b'"' {
                quoted = false;
                previous = Some(byte);
            }
        } else if byte == b'"' {
            quoted = true;
        } else if byte == b',' {
            let next = bytes[i + 1..].iter().find(|b| !b.is_ascii_whitespace());
            if matches!(next, Some(b']' | b'}')) {
                if !matches!(
                    previous,
                    Some(b'"' | b']' | b'}' | b'0'..=b'9' | b'e' | b'l')
                ) {
                    return Err(FileError::InvalidJsonc);
                }
                bytes[i] = b' ';
            }
            previous = Some(byte);
        } else if !byte.is_ascii_whitespace() {
            previous = Some(byte);
        }
        i += 1;
    }
    serde_json::from_slice(&bytes).map_err(|_| FileError::InvalidJsonc)
}
