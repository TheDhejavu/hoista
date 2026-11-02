//! Cloud providers Hoista can deploy to or manage.
//!
//! A provider is identified on the command line and over MCP by a stable,
//! lowercase slug (`cloudflare`, `fly`, ...). Slugs are a compatibility
//! contract: renaming one breaks saved app maps and agent prompts.

use std::fmt;
use std::str::FromStr;

/// A hosting, database or storage provider the user connects with their own
/// account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    /// Workers, Pages, R2 and DNS.
    Cloudflare,
    /// Machines for long-running apps and jobs.
    Fly,
    /// Services imported from existing Railway projects.
    Railway,
    /// Frontends and serverless functions.
    Vercel,
    /// Serverless Postgres.
    Neon,
    /// Postgres with auth and storage.
    Supabase,
}

impl Provider {
    /// Every supported provider, in the order Hoista suggests them.
    pub const ALL: [Provider; 6] = [
        Provider::Cloudflare,
        Provider::Fly,
        Provider::Railway,
        Provider::Vercel,
        Provider::Neon,
        Provider::Supabase,
    ];

    /// Stable identifier used by the CLI, MCP tools and saved app maps.
    pub fn slug(self) -> &'static str {
        match self {
            Provider::Cloudflare => "cloudflare",
            Provider::Fly => "fly",
            Provider::Railway => "railway",
            Provider::Vercel => "vercel",
            Provider::Neon => "neon",
            Provider::Supabase => "supabase",
        }
    }

    /// Name as the provider writes it, for display.
    pub fn display_name(self) -> &'static str {
        match self {
            Provider::Cloudflare => "Cloudflare",
            Provider::Fly => "Fly.io",
            Provider::Railway => "Railway",
            Provider::Vercel => "Vercel",
            Provider::Neon => "Neon",
            Provider::Supabase => "Supabase",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Returned when a slug does not name a supported provider.
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

    /// Parses a slug, ignoring ASCII case. `fly.io` is accepted for Fly.
    ///
    /// # Errors
    ///
    /// Returns [`UnknownProvider`] when the input names no supported provider.
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let wanted = input.trim();
        if wanted.eq_ignore_ascii_case("fly.io") {
            return Ok(Provider::Fly);
        }
        Provider::ALL
            .into_iter()
            .find(|provider| provider.slug().eq_ignore_ascii_case(wanted))
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
