//! GitHub: lists the repositories a token can reach, so a project can be
//! linked to one. GitHub runs nothing itself, so it has no resources.

use super::http::ProviderHttp;
use crate::{
    Provider,
    application::ports::{
        catalog::EngineError,
        providers::{ProviderAdapter, ProviderRepository, RemoteResource},
    },
    domain::{
        Secret,
        model::{github_login as login, github_repository as repository_name, *},
    },
};
use reqwest::Method;
use serde_json::Value;

/// GitHub's page size limit, and the most pages read from one list.
const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 10;
const MAX_BRANCH_CHARS: usize = 255;
const MAX_DESCRIPTION_CHARS: usize = 200;
const COMMIT_SHA_CHARS: usize = 40;

/// GitHub's API adapter.
pub struct GitHub {
    http: ProviderHttp,
}

/// The provider's mark, for pages Hoista serves.
const LOGO: &str = r##"<svg width="26" height="26" viewBox="0 0 24 24"><path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" fill="currentColor"/></svg>"##;

/// GitHub's entry in the provider list. The adapter answers account choices
/// itself (the person, then their organizations); this listing is the
/// organizations part.
pub(crate) const ENTRY: super::ProviderEntry = super::ProviderEntry {
    provider: Provider::Github,
    logo: LOGO,
    capabilities: Capabilities::NONE,
    cli: None,
    api: super::ProviderApi {
        base: "https://api.github.com/",
        hosts: &["api.github.com"],
        explains_refusals: false,
        sign_in: super::SignIn::Bearer,
    },
    accounts: super::accounts::AccountListing {
        fetch: super::accounts::Fetch::Get {
            path: &["user", "orgs"],
            query: &[("per_page", "100")],
            page: None,
            cursor: None,
        },
        rows: &[],
        item: &[],
        id: "login",
        pages: super::accounts::Pages::One,
    },
    adapter: |http| std::sync::Arc::new(GitHub::new(http)),
    #[cfg(test)]
    fake: FAKE,
};

/// A fake of GitHub for the conformance suite: a person with one repository.
#[cfg(test)]
const FAKE: super::conformance::Fake = super::conformance::Fake {
    token: super::conformance::TOKEN,
    account: "sam-rivera",
    resource: "sam-rivera/shop",
    kind: crate::domain::model::ResourceKind::Web,
    routes: &[
        super::conformance::Route {
            path: "/user/orgs",
            body: "",
            answer: r#"[{"login":"sam-builds"}]"#,
        },
        super::conformance::Route {
            path: "/user/repos",
            body: "",
            answer: r#"[{"full_name":"sam-rivera/shop","default_branch":"main","private":true,"pushed_at":"2026-11-01T10:00:00Z","html_url":"https://github.com/sam-rivera/shop","description":null,"owner":{"login":"sam-rivera"}}]"#,
        },
        super::conformance::Route {
            path: "/user",
            body: "",
            answer: r#"{"login":"sam-rivera","name":"Sam Rivera"}"#,
        },
    ],
    empty: "[]",
};

fn invalid() -> EngineError {
    super::answers::incomplete("GitHub")
}

/// A branch name GitHub would accept, without anything that could reach
/// another path.
fn branch_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_BRANCH_CHARS
        && !text.contains("..")
        && !text.starts_with('/')
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
}

/// Splits `owner/name`, refusing anything that isn't a repository name.
fn split_repository(repository: &str) -> Result<(&str, &str), EngineError> {
    repository
        .split_once('/')
        .filter(|_| repository_name(repository))
        .ok_or(EngineError::Invalid("Name the repository as owner/name."))
}

fn repository(row: &Value) -> Option<ProviderRepository> {
    let text = |key: &str| row[key].as_str().filter(|text| !text.is_empty());
    let full_name = text("full_name").filter(|name| repository_name(name))?;
    Some(ProviderRepository {
        full_name: full_name.to_owned(),
        default_branch: text("default_branch").unwrap_or("main").to_owned(),
        private: row["private"].as_bool().unwrap_or(true),
        pushed_at: text("pushed_at").map(str::to_owned),
        url: format!("https://github.com/{full_name}"),
        description: text("description")
            .map(|text| text.chars().take(MAX_DESCRIPTION_CHARS).collect()),
    })
}

impl GitHub {
    /// Shares the allow-listed HTTP boundary.
    pub fn new(http: ProviderHttp) -> Self {
        Self { http }
    }

    async fn get(
        &self,
        token: &Secret,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Value, EngineError> {
        self.http
            .json(Provider::Github, token, Method::GET, path, query, None)
            .await
    }

    /// The person the token belongs to.
    async fn me(&self, token: &Secret) -> Result<String, EngineError> {
        let me = self.get(token, &["user"], &[]).await?;
        me["login"]
            .as_str()
            .filter(|name| login(name))
            .map(str::to_owned)
            .ok_or_else(invalid)
    }

    /// Every page of a list, newest push first, up to `MAX_PAGES` pages.
    async fn pages(&self, token: &Secret, path: &[&str]) -> Result<Vec<Value>, EngineError> {
        let per_page = PER_PAGE.to_string();
        let mut rows = Vec::new();
        for page in 1..=MAX_PAGES {
            let page = page.to_string();
            let query = [
                ("per_page", per_page.as_str()),
                ("sort", "pushed"),
                ("page", &page),
            ];
            let value = self.get(token, path, &query).await?;
            let batch = value.as_array().ok_or_else(invalid)?;
            rows.extend(batch.iter().cloned());
            if batch.len() < PER_PAGE {
                break;
            }
        }
        Ok(rows)
    }
}

#[async_trait::async_trait]
impl ProviderAdapter for GitHub {
    fn provider(&self) -> Provider {
        Provider::Github
    }

    fn capabilities(&self) -> Capabilities {
        ENTRY.capabilities.clone()
    }

    /// The person, then each organization the token can see.
    async fn account_choices(&self, token: &Secret) -> Result<Vec<AccountChoice>, EngineError> {
        let me = self.me(token).await?;
        let mut choices = vec![AccountChoice {
            external_id: me.clone(),
            name: me,
        }];
        // A fine-grained token may not be allowed to list organizations:
        // the person's own account is still there.
        if let Ok(orgs) = self
            .get(
                token,
                &["user", "orgs"],
                &[("per_page", &PER_PAGE.to_string())],
            )
            .await
        {
            for org in orgs.as_array().into_iter().flatten() {
                if let Some(name) = org["login"].as_str().filter(|name| login(name)) {
                    choices.push(AccountChoice {
                        external_id: name.to_owned(),
                        name: name.to_owned(),
                    });
                }
            }
        }
        Ok(choices)
    }

    async fn verify(&self, token: &Secret, owner: &str) -> Result<Vec<String>, EngineError> {
        if !login(owner) {
            return Err(EngineError::Invalid(
                "A GitHub user or organization is required.",
            ));
        }
        if !self
            .account_choices(token)
            .await?
            .iter()
            .any(|choice| choice.external_id.eq_ignore_ascii_case(owner))
        {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.token_scope".into(),
                message: format!("This GitHub token can't reach {owner}."),
            });
        }
        Ok(vec![
            "Reads the repositories it was given (verified by request)".into(),
        ])
    }

    /// GitHub runs nothing, so there's nothing to find.
    async fn discover(
        &self,
        _token: &Secret,
        _owner: &str,
    ) -> Result<Vec<RemoteResource>, EngineError> {
        Ok(Vec::new())
    }

    async fn repositories(
        &self,
        token: &Secret,
        owner: &str,
    ) -> Result<Vec<ProviderRepository>, EngineError> {
        if !login(owner) {
            return Err(invalid());
        }
        let rows = if self.me(token).await?.eq_ignore_ascii_case(owner) {
            // The person's own, and ones they were added to.
            self.pages(token, &["user", "repos"]).await?
        } else {
            self.pages(token, &["orgs", owner, "repos"]).await?
        };
        Ok(rows
            .iter()
            .filter(|row| {
                row["owner"]["login"]
                    .as_str()
                    .is_none_or(|name| name.eq_ignore_ascii_case(owner))
            })
            .filter_map(repository)
            .collect())
    }

    /// The repository's branches, as many as `pages` reads.
    async fn branches(
        &self,
        token: &Secret,
        _owner: &str,
        repository: &str,
    ) -> Result<Vec<String>, EngineError> {
        let (owner, name) = split_repository(repository)?;
        let rows = self
            .pages(token, &["repos", owner, name, "branches"])
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| row["name"].as_str().filter(|name| branch_name(name)))
            .map(str::to_owned)
            .collect())
    }

    async fn branch_head(
        &self,
        token: &Secret,
        _owner: &str,
        repository: &str,
        branch: &str,
    ) -> Result<String, EngineError> {
        let (owner, name) = split_repository(repository)?;
        if !branch_name(branch) {
            return Err(EngineError::Invalid("That isn't a branch name."));
        }
        let commit = self
            .get(token, &["repos", owner, name, "commits", branch], &[])
            .await?;
        commit["sha"]
            .as_str()
            .filter(|sha| {
                sha.len() == COMMIT_SHA_CHARS && sha.bytes().all(|b| b.is_ascii_hexdigit())
            })
            .map(str::to_owned)
            .ok_or_else(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_githubs() {
        assert!(repository_name("TheDhejavu/vercel-custom-server"));
        assert!(repository_name("sam/site.io"));
        assert!(!repository_name("sam/../x"));
        assert!(!repository_name("sam"));
        assert!(!repository_name("-sam/x"));
        assert!(branch_name("feature/one"));
        assert!(!branch_name("../main"));
    }

    #[test]
    fn repositories_read_githubs_rows() {
        let row = serde_json::json!({
            "full_name": "sam/shop",
            "default_branch": "trunk",
            "private": false,
            "pushed_at": "2026-11-01T10:00:00Z",
        });
        let repo = repository(&row).unwrap();
        assert_eq!(repo.default_branch, "trunk");
        assert_eq!(repo.url, "https://github.com/sam/shop");
        assert!(repository(&serde_json::json!({"full_name":"bad name"})).is_none());
    }
}
