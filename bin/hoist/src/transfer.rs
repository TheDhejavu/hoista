//! Moving Hoista between computers in one encrypted file. Passphrases are
//! typed hidden or piped; they never go in a command's arguments.

use anyhow::{Context, bail};
use hoista_server::client::LocalClient;
use std::io::{IsTerminal, Read};
use zeroize::{Zeroize, Zeroizing};

/// Passphrase length in bytes, as the transfer encryption requires.
const PASSPHRASE_BYTES: std::ops::RangeInclusive<usize> = 12..=1024;
pub(crate) async fn run(
    client: &LocalClient,
    operation: &str,
    path: &str,
    stdin: bool,
) -> anyhow::Result<()> {
    let passphrase = if stdin {
        if std::io::stdin().is_terminal() {
            bail!(
                "Pipe the exact passphrase bytes with --passphrase-stdin, or use hidden terminal entry."
            );
        }
        let mut bytes = Zeroizing::new(Vec::new());
        std::io::stdin()
            .take(*PASSPHRASE_BYTES.end() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > *PASSPHRASE_BYTES.end() {
            bail!("The passphrase exceeds 1024 UTF-8 bytes.");
        }
        Zeroizing::new(
            std::str::from_utf8(&bytes)
                .context("Passphrase must be UTF-8")?
                .to_owned(),
        )
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("Use --passphrase-stdin for exact piped input, or run in a terminal.");
        }
        let value = Zeroizing::new(rpassword::prompt_password(
            "Transfer passphrase (hidden): ",
        )?);
        if operation == "export" {
            let confirmation = Zeroizing::new(rpassword::prompt_password("Repeat passphrase: ")?);
            if *value != *confirmation {
                bail!("The passphrases do not match.");
            }
        }
        value
    };
    if !PASSPHRASE_BYTES.contains(&passphrase.len()) {
        bail!("Use a passphrase of 12–1024 UTF-8 bytes.");
    }
    let path = std::path::absolute(path)?;
    let body = SensitiveBody(serde_json::json!({"path":path,"passphrase":passphrase.as_str()}));
    let response = client
        .post_json(&format!("/transfer/{operation}"), &body.0)
        .await;
    if response.is_err() && operation == "restore" {
        eprintln!(
            "A restore can finish after the connection drops. Run hoist backups recovery before trying again; don't delete state or keychain entries."
        );
    }
    if response.is_err() && operation == "export" {
        eprintln!(
            "An export can finish after the connection drops. Look for the file before trying again; existing files are never overwritten."
        );
    }
    let response = response?;
    crate::ui::show(&response, |response| {
        if operation == "inspect" {
            println!(
                "{}",
                serde_json::to_string_pretty(response).unwrap_or_default()
            );
        }
        crate::ui::done(match operation {
            "export" => {
                "Exported. Keep the passphrase somewhere safe; the file is useless without it."
            }
            "inspect" => "The file opens with that passphrase.",
            _ => {
                "Restored. Projects start as watching; manage them again with hoist projects manage."
            }
        });
    })
}

struct SensitiveBody(serde_json::Value);
impl Drop for SensitiveBody {
    fn drop(&mut self) {
        if let serde_json::Value::String(value) = &mut self.0["passphrase"] {
            value.zeroize();
        }
    }
}
