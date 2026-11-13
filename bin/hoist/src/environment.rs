//! Reading a variable's value without it passing through a command's arguments.

use anyhow::{Context, bail};
use hoista_server::domain::Secret;
use std::io::{IsTerminal, Read};
use zeroize::Zeroizing;

const VALUE_LIMIT: usize = 16 * 1024;

/// One value from `input`: UTF-8, at most 16 KiB, no NUL. Kept exactly,
/// newlines included.
pub(crate) fn read_value(input: impl Read) -> anyhow::Result<Secret> {
    let mut bytes = Zeroizing::new(Vec::new());
    input
        .take((VALUE_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Couldn't read the value."))?;
    if bytes.len() > VALUE_LIMIT {
        bail!("The value is over 16 KiB.");
    }
    let text = std::str::from_utf8(&bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
        .context("Give a UTF-8 value of at most 16 KiB without NUL.")?;
    Ok(Secret::new(text.to_owned()))
}

/// A value from stdin, or typed at a hidden prompt.
pub(crate) fn value(stdin: bool, prompt: &str) -> anyhow::Result<Secret> {
    if stdin {
        require_pipe()?;
        return read_value(std::io::stdin());
    }
    if !std::io::stdin().is_terminal() {
        bail!("Pipe the value with --value-stdin, or run this in a terminal.");
    }
    let typed = Zeroizing::new(
        rpassword::prompt_password(format!("{prompt}: "))
            .context("Couldn't read the hidden prompt")?,
    );
    read_value(typed.as_bytes())
}

/// Stops when stdin is a terminal: the input has to be piped.
pub(crate) fn require_pipe() -> anyhow::Result<()> {
    if std::io::stdin().is_terminal() {
        bail!(
            "Pipe the input, from your password manager or a private file. Values never go in a command's arguments."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_keep_their_bytes_and_bad_input_is_refused() {
        for value in ["", "one\ntwo\n", "  spaced  ", "unicode-é"] {
            assert_eq!(read_value(value.as_bytes()).unwrap().expose(), value);
        }
        assert!(read_value(&[0xff][..]).is_err());
        assert!(read_value(&b"a\0b"[..]).is_err());
        assert!(read_value("a".repeat(VALUE_LIMIT + 1).as_bytes()).is_err());
    }
}
