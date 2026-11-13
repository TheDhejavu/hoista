//! The providers Hoista works with.
//!
//! Each has a lowercase slug (`cloudflare`, `fly`, ...) that the CLI, MCP and
//! saved data use. Never rename one: saved projects and agents' prompts name it.

use std::fmt;
use std::str::FromStr;

/// Declares every provider in one row each: variant, slug, the name the provider
/// writes, what the provider calls the account a token belongs to, and the pages
/// on its website where people make an account and a token.
///
/// Adding a provider is one row here, one adapter file under
/// `crates/providers/src/`, and one line in its `ENTRIES` list.
macro_rules! providers {
    ($($(#[doc = $doc:literal])* $variant:ident = $slug:literal, $name:literal, $account:literal,
        signup: $signup:literal, tokens: $tokens:literal;)+) => {
        /// A provider the person connects with an account of their own.
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            Hash,
            serde::Serialize,
            serde::Deserialize,
            schemars::JsonSchema,
            ts_rs::TS,
        )]
        #[serde(rename_all = "snake_case")]
        pub enum Provider {
            $($(#[doc = $doc])* $variant,)+
        }

        impl Provider {
            /// Every provider, in the order Hoista suggests them.
            pub const ALL: [Provider; [$($slug),+].len()] = [$(Provider::$variant),+];

            /// The slug: how the CLI, MCP and saved data name it.
            pub fn slug(self) -> &'static str {
                match self {
                    $(Provider::$variant => $slug,)+
                }
            }

            /// The name as the provider writes it.
            pub fn display_name(self) -> &'static str {
                match self {
                    $(Provider::$variant => $name,)+
                }
            }

            /// What the provider calls the account a token belongs to, in its own
            /// words: `account ID`, `organization slug`, `workspace ID`.
            pub fn account_label(self) -> &'static str {
                match self {
                    $(Provider::$variant => $account,)+
                }
            }

            /// The provider's page for making an account.
            pub fn signup_page(self) -> &'static str {
                match self {
                    $(Provider::$variant => $signup,)+
                }
            }

            /// The provider's page for making the token Hoista asks for.
            pub fn token_page(self) -> &'static str {
                match self {
                    $(Provider::$variant => $tokens,)+
                }
            }
        }
    };
}

providers! {
    /// Workers, Pages, R2 and DNS.
    Cloudflare = "cloudflare", "Cloudflare", "account ID",
        signup: "https://dash.cloudflare.com/sign-up",
        tokens: "https://dash.cloudflare.com/profile/api-tokens";
    /// Machines for long-running projects and jobs.
    Fly = "fly", "Fly.io", "organization slug",
        signup: "https://fly.io/app/sign-up",
        tokens: "https://fly.io/dashboard/personal/tokens";
    /// Services, workers and databases in Railway projects.
    Railway = "railway", "Railway", "workspace ID",
        signup: "https://railway.com/login",
        tokens: "https://railway.com/account/tokens";
    /// Frontends and serverless functions.
    Vercel = "vercel", "Vercel", "team ID",
        signup: "https://vercel.com/signup",
        tokens: "https://vercel.com/account/settings/tokens";
    /// Serverless Postgres.
    Neon = "neon", "Neon", "organization ID",
        signup: "https://console.neon.tech/signup",
        tokens: "https://console.neon.tech/app/settings#api-keys";
    /// Postgres with auth and storage.
    Supabase = "supabase", "Supabase", "organization ID",
        signup: "https://supabase.com/dashboard/sign-up",
        tokens: "https://supabase.com/dashboard/account/tokens";
    /// Render: web services, workers, cron jobs, Postgres and Key Value.
    Render = "render", "Render", "workspace ID",
        signup: "https://dashboard.render.com/register",
        tokens: "https://dashboard.render.com/u/settings?add-api-key";
    /// Resend: email sending domains.
    Resend = "resend", "Resend", "team",
        signup: "https://resend.com/signup",
        tokens: "https://resend.com/api-keys";
    /// Runpod: GPU and CPU pods, Serverless endpoints and network volumes.
    Runpod = "runpod", "Runpod", "account",
        signup: "https://console.runpod.io/signup",
        tokens: "https://console.runpod.io/user/settings";
    /// Modal: serverless GPU apps and their volumes.
    Modal = "modal", "Modal", "workspace",
        signup: "https://modal.com/signup",
        tokens: "https://modal.com/settings/tokens";
    /// GitHub: the repositories projects are built from. It runs nothing itself.
    Github = "github", "GitHub", "account",
        signup: "https://github.com/signup",
        tokens: "https://github.com/settings/personal-access-tokens/new";
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// A name that isn't one of Hoista's providers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProvider(String);

impl fmt::Display for UnknownProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown provider `{}`", self.0)
    }
}

impl std::error::Error for UnknownProvider {}

impl FromStr for Provider {
    type Err = UnknownProvider;

    /// Parses a slug or the provider's own name, ignoring ASCII case (`fly` or `Fly.io`).
    ///
    /// # Errors
    ///
    /// Returns [`UnknownProvider`] when it names none of them.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let wanted = input.trim();
        Provider::ALL
            .into_iter()
            .find(|provider| {
                provider.slug().eq_ignore_ascii_case(wanted)
                    || provider.display_name().eq_ignore_ascii_case(wanted)
            })
            .ok_or_else(|| UnknownProvider(wanted.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slug_parses_back_to_its_provider() {
        for provider in Provider::ALL {
            assert_eq!(provider.slug().parse::<Provider>(), Ok(provider));
        }
    }

    #[test]
    fn every_provider_links_to_its_own_https_pages() {
        for provider in Provider::ALL {
            assert!(provider.signup_page().starts_with("https://"));
            assert!(provider.token_page().starts_with("https://"));
        }
    }

    #[test]
    fn slugs_are_unique_and_lowercase() {
        for (index, provider) in Provider::ALL.iter().enumerate() {
            assert_eq!(provider.slug(), provider.slug().to_ascii_lowercase());
            assert!(
                Provider::ALL[index + 1..]
                    .iter()
                    .all(|other| other.slug() != provider.slug())
            );
        }
    }

    #[test]
    fn parsing_ignores_case_and_accepts_the_fly_domain() {
        assert_eq!("Cloudflare".parse::<Provider>(), Ok(Provider::Cloudflare));
        assert_eq!("fly.io".parse::<Provider>(), Ok(Provider::Fly));
    }

    #[test]
    fn unknown_slugs_are_reported_by_name() {
        let error = "heroku".parse::<Provider>().unwrap_err();
        assert_eq!(error.to_string(), "unknown provider `heroku`");
    }
}
