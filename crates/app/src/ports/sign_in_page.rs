//! The page a provider's sign-in returns the browser to. Hoista's use cases
//! say what happened; how it looks is the implementation's business.

/// Shows the browser how a provider sign-in ended.
pub trait SignInPage: Send + Sync {
    /// A whole HTML page: the provider's logo (trusted markup) beside
    /// Hoista's, ticked when it `connected`, then `title` and `text`.
    fn render(
        &self,
        provider: &str,
        logo: &str,
        connected: bool,
        title: &str,
        text: &str,
    ) -> String;
}
