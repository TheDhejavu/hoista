//! `hoist accounts` and `hoist providers`: connecting provider accounts, checking
//! them, and rotating their tokens.
//!
//! Tokens are never taken as arguments, only from a hidden prompt or
//! `--token-stdin`, so they stay out of shell history and process lists.

use crate::{
    context::{self, Context},
    loopback::Loopback,
    ui,
};
use anyhow::{Context as _, bail};
use clap::{Args, Subcommand};
use hoista_server::{Provider, client::LocalClient, domain::Secret};
use serde_json::{Value, json};
use std::{
    io::{IsTerminal, Read},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_LABEL_BYTES: usize = 80;
const MAX_ID_BYTES: usize = 256;
/// How long the person has to finish a provider's sign-in page.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct AccountsArgs {
    #[command(subcommand)]
    command: Option<AccountsCommand>,
}

#[derive(Subcommand)]
enum AccountsCommand {
    /// Connect a provider account. In a terminal, asks for anything not given.
    Add(AddArgs),
    /// Check an account's token still works at its provider.
    Check {
        /// Account name or id.
        account: Option<String>,
    },
    /// Look for projects on an account now.
    Refresh {
        /// Account name or id.
        account: Option<String>,
    },
    /// Replace an account's token with a new one. The new token is checked first.
    Rotate {
        /// Account name or id.
        account: Option<String>,
        /// Read the new token from stdin.
        #[arg(long)]
        token_stdin: bool,
    },
    /// Disconnect an account. What runs at the provider is untouched.
    Remove {
        /// Account name or id.
        account: Option<String>,
    },
}

#[derive(Args)]
struct AddArgs {
    /// The provider: cloudflare, fly, railway, vercel, neon, supabase, render, resend, runpod, modal, github.
    provider: Option<String>,
    /// The provider's id for the account (account ID, team ID, organization slug). Found from the token when left out.
    #[arg(long, value_name = "ID")]
    account: Option<String>,
    /// What to call it in Hoista. Default: the provider's name for it.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Read the token from stdin instead of signing in or a hidden prompt.
    #[arg(long)]
    token_stdin: bool,
}

fn parse_provider(name: &str) -> anyhow::Result<Provider> {
    name.parse::<Provider>().or_else(|_| {
        ui::usage(format!(
            "Hoista doesn't work with {} yet. Providers: {}.",
            ui::clean(name),
            Provider::ALL
                .iter()
                .map(|provider| provider.slug())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

/// What each provider can do, from the engine.
async fn capabilities(client: &LocalClient) -> anyhow::Result<Vec<Value>> {
    let value = client.get_json("/providers").await?;
    Ok(value["items"].as_array().cloned().unwrap_or_default())
}

fn signs_in(providers: &[Value], provider: Provider) -> bool {
    providers.iter().any(|item| {
        item["provider"] == provider.slug() && item["capabilities"]["browser_sign_in"] == true
    })
}

pub(crate) async fn providers(context: &Context) -> anyhow::Result<()> {
    let client = context.client().await?;
    let providers = capabilities(&client).await?;
    ui::list(
        &Value::Array(providers),
        "No providers are available.",
        |item| {
            let connects = if item["capabilities"]["browser_sign_in"] == true {
                "sign in or token"
            } else {
                "token"
            };
            vec![
                ui::text(item, "provider"),
                ui::text(item, "name"),
                connects.into(),
                ui::text(item, "account_label"),
            ]
        },
        &["PROVIDER", "NAME", "CONNECTS WITH", "ACCOUNT IS A"],
    )
}

async fn all(client: &LocalClient) -> anyhow::Result<Vec<Value>> {
    let mut items = Vec::new();
    let mut path = "/accounts".to_owned();
    for _ in 0..100 {
        let page = client.get_json(&path).await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        let Some(cursor) = page["next_cursor"].as_str() else {
            return Ok(items);
        };
        path = format!("/accounts?{}", context::query(&[("cursor", cursor)]));
    }
    bail!("The account list didn't end.")
}

/// An account by name (label), id or the provider's id for it, or picked.
pub(crate) async fn account(
    client: &LocalClient,
    wanted: Option<&str>,
    only: Option<&dyn Fn(&Value) -> bool>,
    how: &str,
) -> anyhow::Result<Value> {
    let accounts: Vec<Value> = all(client)
        .await?
        .into_iter()
        .filter(|account| only.is_none_or(|keep| keep(account)))
        .collect();
    if let Some(wanted) = wanted {
        let found: Vec<_> = accounts
            .iter()
            .filter(|account| {
                account["id"] == wanted
                    || account["label"]
                        .as_str()
                        .is_some_and(|label| label.eq_ignore_ascii_case(wanted))
                    || account["external_id"] == wanted
            })
            .collect();
        return match found.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!(
                "There's no account called {}. Run hoist accounts.",
                ui::clean(wanted)
            ),
            _ => bail!(
                "More than one account is called {}. Use its id.",
                ui::clean(wanted)
            ),
        };
    }
    match accounts.as_slice() {
        [] => bail!("No accounts are connected that fit. Add one with hoist accounts add."),
        [one] if !ui::can_ask() => Ok(one.clone()),
        _ if ui::can_ask() => {
            let names: Vec<String> = accounts.iter().map(describe).collect();
            Ok(accounts[ui::pick("Account", &names)?].clone())
        }
        _ => ui::usage(how.to_owned()),
    }
}

fn describe(account: &Value) -> String {
    format!(
        "{} ({}, {})",
        ui::text(account, "label"),
        ui::text(account, "provider"),
        ui::text(account, "external_id")
    )
}

pub(crate) async fn accounts(context: &Context, args: AccountsArgs) -> anyhow::Result<()> {
    if let Some(AccountsCommand::Add(args)) = args.command {
        return add(context, args).await;
    }
    let client = context.client().await?;
    match args.command {
        None => ui::list(
            &Value::Array(all(&client).await?),
            "No accounts yet. Connect one with hoist accounts add.",
            |item| {
                vec![
                    ui::text(item, "label"),
                    ui::text(item, "provider"),
                    ui::text(item, "external_id"),
                    ui::text(item, "status"),
                    ui::when(item, "connected_at"),
                    ui::text(item, "id"),
                ]
            },
            &["NAME", "PROVIDER", "ACCOUNT", "STATUS", "CONNECTED", "ID"],
        ),
        Some(AccountsCommand::Add(_)) => unreachable!("handled above"),
        Some(AccountsCommand::Check { account }) => {
            let account = self::account(
                &client,
                account.as_deref(),
                None,
                "Name the account: hoist accounts check ACCOUNT.",
            )
            .await?;
            let checked = client
                .post_json(
                    &context::path(&["accounts", ui::text(&account, "id").as_str(), "check"]),
                    &json!({}),
                )
                .await?;
            ui::show(&checked, |checked| {
                if checked["status"] == "connected" {
                    ui::done(&format!("{} works.", ui::text(checked, "label")));
                } else {
                    ui::warn(&format!(
                        "{} is {}. Rotate its token with hoist accounts rotate {}.",
                        ui::text(checked, "label"),
                        ui::text(checked, "status"),
                        ui::text(checked, "label")
                    ));
                }
            })
        }
        Some(AccountsCommand::Refresh { account }) => {
            let account = self::account(
                &client,
                account.as_deref(),
                None,
                "Name the account: hoist accounts refresh ACCOUNT.",
            )
            .await?;
            ui::note(&format!(
                "Looking for projects on {}",
                ui::text(&account, "label")
            ));
            let found = client
                .post_json("/import/scan", &json!({"account_id": account["id"]}))
                .await?;
            ui::show(&found, |found| {
                ui::done(&format!("Looked. {}", summary(found)));
                ui::note("Bring what it found in with hoist import.");
            })
        }
        Some(AccountsCommand::Rotate {
            account,
            token_stdin,
        }) => {
            let account = self::account(
                &client,
                account.as_deref(),
                None,
                "Name the account: hoist accounts rotate ACCOUNT --token-stdin.",
            )
            .await?;
            let provider = parse_provider(&ui::text(&account, "provider"))?;
            if !token_stdin {
                ui::note(&format!("Make a new token at {}", provider.token_page()));
            }
            let token = read_secret(token_stdin, "New token (hidden)")?;
            let saved = save(
                &client,
                provider,
                &ui::text(&account, "external_id"),
                &ui::text(&account, "label"),
                &token,
            )
            .await?;
            ui::show(&saved, |saved| {
                ui::done(&format!(
                    "{}'s token is replaced. Revoke the old one at the provider.",
                    ui::text(saved, "label")
                ));
            })
        }
        Some(AccountsCommand::Remove { account }) => {
            let account = self::account(
                &client,
                account.as_deref(),
                None,
                "Name the account: hoist accounts remove ACCOUNT --yes.",
            )
            .await?;
            if !ui::confirm(
                &format!(
                    "Disconnect {}? Hoista stops watching what runs there; nothing at the provider changes.",
                    describe(&account)
                ),
                false,
            )? {
                return Ok(());
            }
            let removed = client
                .delete_json(&context::path(&[
                    "accounts",
                    ui::text(&account, "id").as_str(),
                ]))
                .await?;
            ui::show(&removed, |_| {
                ui::done("Disconnected. Its token is gone from this computer's keychain.")
            })
        }
    }
}

/// "Found 12 resources." from a scan's result or an account's last one.
fn summary(found: &Value) -> String {
    let count = found["items"]
        .as_array()
        .map(|items| items.len() as u64)
        .or_else(|| found["last_discovery"]["observed_resources"].as_u64());
    let complete = found["complete"]
        .as_bool()
        .or_else(|| found["last_discovery"]["complete"].as_bool())
        .unwrap_or(true);
    match count {
        Some(count) if complete => format!("Found {count} resources."),
        Some(count) => {
            format!("Found {count} resources so far; the provider didn't list everything.")
        }
        None => String::new(),
    }
}

/// Reads a token: from stdin with `--token-stdin`, otherwise at a hidden prompt.
pub(crate) fn read_secret(stdin: bool, prompt: &str) -> anyhow::Result<Secret> {
    if stdin {
        if std::io::stdin().is_terminal() {
            return ui::usage(
                "--token-stdin reads a pipe, like: op read op://vault/item/token | hoist accounts add fly --token-stdin",
            );
        }
        return read_token(std::io::stdin());
    }
    if !ui::can_ask() {
        return ui::usage(
            "Pipe the token with --token-stdin. Tokens never go in a command's arguments.",
        );
    }
    let typed = Zeroizing::new(
        rpassword::prompt_password(format!("{prompt}: "))
            .context("Couldn't read the hidden prompt")?,
    );
    read_token(typed.as_bytes())
}

/// One token: at most 16 KiB of UTF-8, one line, no control characters.
pub(crate) fn read_token(input: impl Read) -> anyhow::Result<Secret> {
    let mut bytes = Zeroizing::new(Vec::new());
    input
        // Room for a trailing CRLF, and one byte more to tell a token that's too long.
        .take(MAX_TOKEN_BYTES as u64 + 3)
        .read_to_end(&mut bytes)
        .context("Couldn't read the token")?;
    // A password manager's trailing newline, not other whitespace.
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    let text = std::str::from_utf8(&bytes).ok().filter(|text| {
        !text.is_empty() && text.len() <= MAX_TOKEN_BYTES && !text.chars().any(char::is_control)
    });
    let text =
        text.context("Give one token: up to 16 KiB on one line, without control characters")?;
    Ok(Secret::new(text.to_owned()))
}

/// Sends a token in a request body and clears it from memory afterwards.
async fn with_token(client: &LocalClient, path: &str, mut body: Value) -> anyhow::Result<Value> {
    let result = client.post_json(path, &body).await;
    if let Value::String(token) = &mut body["token"] {
        token.zeroize();
    }
    Ok(result?)
}

async fn save(
    client: &LocalClient,
    provider: Provider,
    external_id: &str,
    label: &str,
    token: &Secret,
) -> anyhow::Result<Value> {
    ui::note(&format!(
        "Checking the token with {}",
        provider.display_name()
    ));
    with_token(
        client,
        "/accounts",
        json!({
            "provider": provider,
            "external_id": external_id,
            "label": label,
            "token": token.expose(),
        }),
    )
    .await
}

async fn add(context: &Context, args: AddArgs) -> anyhow::Result<()> {
    let provider = match args.provider {
        Some(name) => parse_provider(&name)?,
        None if ui::can_ask() => {
            let names: Vec<String> = Provider::ALL
                .iter()
                .map(|provider| provider.display_name().to_owned())
                .collect();
            Provider::ALL[ui::pick("Provider", &names)?]
        }
        None => {
            return ui::usage(
                "Name the provider: hoist accounts add PROVIDER --token-stdin. See hoist providers.",
            );
        }
    };
    // With a token already chosen, read and check it before starting the
    // daemon, so a bad paste never touches state.
    let early = if args.token_stdin || args.account.is_some() {
        Some(token_for(provider, args.token_stdin)?)
    } else {
        None
    };
    let client = &context.client().await?;
    let providers = capabilities(client).await?;
    let browser = early.is_none()
        && signs_in(&providers, provider)
        && ui::can_ask()
        && args.account.is_none()
        && ui::pick(
            "Connect with",
            &[
                format!("Sign in on {}'s page", provider.display_name()),
                "Paste a token".to_owned(),
            ],
        )? == 0;
    let saved = if browser {
        sign_in(client, provider).await?
    } else {
        let token = match early {
            Some(token) => token,
            None => token_for(provider, args.token_stdin)?,
        };
        let (external_id, suggested) = match args.account {
            Some(id) => (id.clone(), id),
            None => choose_account(client, provider, &token).await?,
        };
        let label = match args.name {
            Some(name) => name,
            None if ui::can_ask() => ui::ask("Name in Hoista", Some(&suggested))?,
            None => suggested,
        };
        if label.is_empty()
            || label.len() > MAX_LABEL_BYTES
            || external_id.is_empty()
            || external_id.len() > MAX_ID_BYTES
        {
            return ui::usage("Use a name of 1 to 80 characters and an account id of up to 256.");
        }
        save(client, provider, &external_id, &label, &token).await?
    };
    ui::show(&saved, |saved| {
        ui::done(&format!(
            "Connected {}. {}",
            ui::text(saved, "label"),
            summary(saved)
        ));
        ui::note("Bring its projects in with hoist import.");
    })
}

/// Reads the token for `provider`, saying where to make one when prompting.
fn token_for(provider: Provider, stdin: bool) -> anyhow::Result<Secret> {
    if !stdin {
        ui::note(&format!(
            "Make a token at {} (Hoista needs read and deploy access).",
            provider.token_page()
        ));
    }
    read_secret(
        stdin,
        &format!("{} token (hidden)", provider.display_name()),
    )
}

/// Which account the token is for: the only one, or picked.
async fn choose_account(
    client: &LocalClient,
    provider: Provider,
    token: &Secret,
) -> anyhow::Result<(String, String)> {
    let found = with_token(
        client,
        "/accounts/lookup",
        json!({"provider": provider, "token": token.expose()}),
    )
    .await
    .unwrap_or(Value::Array(vec![]));
    let choices = found.as_array().cloned().unwrap_or_default();
    let pair = |choice: &Value| {
        (
            choice["external_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            choice["name"].as_str().unwrap_or_default().to_owned(),
        )
    };
    match choices.as_slice() {
        [one] => Ok(pair(one)),
        [] if ui::can_ask() => {
            let id = ui::ask(
                &format!("{} {}", provider.display_name(), provider.account_label()),
                None,
            )?;
            Ok((id.clone(), id))
        }
        [] => ui::usage(format!(
            "Couldn't tell which account the token is for. Pass --account with its {}.",
            provider.account_label()
        )),
        _ if ui::can_ask() => {
            let names: Vec<String> = choices
                .iter()
                .map(|choice| {
                    format!(
                        "{} ({})",
                        ui::text(choice, "name"),
                        ui::text(choice, "external_id")
                    )
                })
                .collect();
            Ok(pair(&choices[ui::pick("Account", &names)?]))
        }
        _ => ui::usage(format!(
            "The token reaches more than one account. Pass --account with one of: {}.",
            choices
                .iter()
                .map(|choice| ui::text(choice, "external_id"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Signs in on the provider's page. On a remote, this computer listens for the
/// browser coming back and forwards it, since the remote can't.
async fn sign_in(client: &LocalClient, provider: Provider) -> anyhow::Result<Value> {
    let relay = client.is_on_server();
    let started = client
        .post_json(
            "/accounts/sign-in",
            &json!({"provider": provider, "relay": relay}),
        )
        .await?;
    let id = started["id"]
        .as_str()
        .context("The sign-in didn't start")?
        .to_owned();
    let url = started["url"]
        .as_str()
        .context("The sign-in didn't start")?
        .to_owned();
    let finish_path = context::path(&["accounts", "sign-in", &id, "finish"]);
    let empty = json!({});
    let finish = client.post_json_long_running(&finish_path, &empty);
    ui::note(&format!(
        "Allow Hoista on {}'s page. If it doesn't open, open: {}",
        provider.display_name(),
        ui::clean(&url)
    ));
    let saved = if relay {
        let port = started["relay_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .context("The remote didn't say where the provider sends the browser back")?;
        let listening = Loopback::bind(port).await?;
        ui::open(&url);
        let forward = listening.wait("/callback", SIGN_IN_TIMEOUT, |params| {
            let id = id.clone();
            async move {
                let code = params.get("code").cloned();
                let mut body = json!({"state": params.get("state").cloned().unwrap_or_default()});
                if let Some(code) = &code {
                    body["code"] = json!(code);
                }
                match client
                    .post_json(
                        &context::path(&["accounts", "sign-in", &id, "callback"]),
                        &body,
                    )
                    .await
                {
                    Ok(_) if code.is_some() => (
                        true,
                        "Go back to the terminal. You can close this tab.".to_owned(),
                    ),
                    Ok(_) => (
                        false,
                        "Nothing was saved. Go back to the terminal to try again.".to_owned(),
                    ),
                    Err(error) => (false, error.to_string()),
                }
            }
        });
        let (forwarded, saved) = tokio::join!(forward, finish);
        forwarded?;
        saved?
    } else {
        ui::open(&url);
        finish.await?
    };
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_one_bounded_line_and_never_echoed() {
        assert_eq!(
            read_token(&b"test-token\r\n"[..]).unwrap().expose(),
            "test-token"
        );
        for bytes in [
            vec![],
            b"test-secret\nsecond".to_vec(),
            vec![b'x'; 16 * 1024 + 1],
            vec![255],
        ] {
            let error = read_token(bytes.as_slice()).unwrap_err().to_string();
            assert!(!error.contains("test-secret"));
        }
        assert!(read_token(vec![b'x'; 16 * 1024].as_slice()).is_ok());
    }

    #[test]
    fn every_provider_parses_by_slug_or_name() {
        for provider in Provider::ALL {
            assert_eq!(parse_provider(provider.slug()).unwrap(), provider);
            assert_eq!(parse_provider(provider.display_name()).unwrap(), provider);
        }
        assert!(parse_provider("heroku").is_err());
    }
}
