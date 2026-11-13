//! Reads a .env file so Hoista can send its values straight to a provider. The
//! values are never returned, logged or put in an error message: an agent
//! names the file, and the secrets never pass through it.

use crate::{application::ports::catalog::EngineError, domain::Secret};

/// The largest .env file Hoista reads.
const MOST: u64 = 1024 * 1024;

/// The variables in the file at `path`, or only those in `names`.
///
/// # Errors
/// A relative path, a file that can't be read or is too big, a line that
/// isn't `NAME=value`, or a name asked for that the file doesn't have. The
/// messages name the variables and lines, never a value.
pub fn read_env_file(
    path: &str,
    names: Option<&[String]>,
) -> Result<Vec<(String, Secret)>, EngineError> {
    let path = std::path::Path::new(path);
    if !path.is_absolute() {
        return Err(EngineError::Invalid("Give the .env file's absolute path."));
    }
    let size = std::fs::metadata(path)
        .map_err(|_| EngineError::Invalid("Hoista can't read that .env file."))?
        .len();
    if size > MOST {
        return Err(EngineError::Invalid("That .env file is over 1 MB."));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|_| EngineError::Invalid("Hoista can't read that .env file as text."))?;
    let mut found = parse(&text).map_err(|line| EngineError::Provider {
        evidence: None,
        code: "env_file.invalid".into(),
        message: format!("Line {line} of the .env file isn't NAME=value."),
    })?;
    if let Some(names) = names {
        let missing: Vec<&str> = names
            .iter()
            .filter(|name| !found.iter().any(|(known, _)| known == *name))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(EngineError::Provider {
                evidence: None,
                code: "env_file.missing".into(),
                message: format!("The .env file has no {}.", missing.join(", ")),
            });
        }
        found.retain(|(name, _)| names.contains(name));
    }
    if found.is_empty() {
        return Err(EngineError::Invalid("That .env file has no variables."));
    }
    Ok(found)
}

/// `NAME=value` lines, skipping blanks and `#` comments, with an optional
/// `export `, and single or double quotes. A later line wins. Errs with the
/// number of the first line it can't read.
fn parse(text: &str) -> Result<Vec<(String, Secret)>, usize> {
    let mut found: Vec<(String, Secret)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=').ok_or(index + 1)?;
        let name = name.trim();
        let ok = !name.is_empty()
            && name.bytes().enumerate().all(|(at, b)| {
                b == b'_' || b.is_ascii_alphabetic() || (at > 0 && b.is_ascii_digit())
            });
        if !ok {
            return Err(index + 1);
        }
        let value = unquote(value.trim());
        found.retain(|(known, _)| known != name);
        found.push((name.to_owned(), Secret::new(value)));
    }
    Ok(found)
}

fn unquote(value: &str) -> String {
    let quoted = |mark: char| value.len() >= 2 && value.starts_with(mark) && value.ends_with(mark);
    if quoted('"') {
        value[1..value.len() - 1]
            .replace("\\n", "\n")
            .replace("\\\"", "\"")
    } else if quoted('\'') {
        value[1..value.len() - 1].to_owned()
    } else {
        // An unquoted value ends at a ` #` comment.
        value
            .split_once(" #")
            .map_or(value, |(before, _)| before)
            .trim_end()
            .to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Vec<(String, String)> {
        parse(text)
            .expect("parses")
            .into_iter()
            .map(|(name, value)| (name, value.expose().to_owned()))
            .collect()
    }

    #[test]
    fn reads_the_usual_shapes() {
        let text = "# keys\n\nexport A=1\nB=\"two words\"\nC='x#y'\nD=plain # note\nE=\"line\\nbreak\"\nA=3\n";
        assert_eq!(
            read(text),
            [
                ("B".into(), "two words".into()),
                ("C".into(), "x#y".into()),
                ("D".into(), "plain".into()),
                ("E".into(), "line\nbreak".into()),
                ("A".into(), "3".into()),
            ]
        );
    }

    #[test]
    fn a_bad_line_is_named_by_number() {
        assert_eq!(parse("A=1\nnot a line\n").err(), Some(2));
        assert_eq!(parse("1A=x").err(), Some(1));
    }

    #[test]
    fn names_pick_some_and_a_missing_one_is_named_without_values() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join(".env");
        std::fs::write(&path, "API_TOKEN=very-secret\nOTHER=1\n").expect("write");
        let path = path.to_str().expect("utf-8");
        let picked = read_env_file(path, Some(&["API_TOKEN".into()])).expect("read");
        assert_eq!(picked.len(), 1);
        let error = read_env_file(path, Some(&["NOPE".into()])).expect_err("missing");
        let message = format!("{error:?}");
        assert!(message.contains("NOPE"));
        assert!(!message.contains("very-secret"));
        assert!(read_env_file(".env", None).is_err());
    }
}
