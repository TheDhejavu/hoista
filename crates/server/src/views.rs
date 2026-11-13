//! The pages Hoista shows in a browser. Each is a template in `templates/`,
//! checked when Hoista is compiled; the templates escape every value, and only
//! drawings (provider logos) are passed through as markup.

use askama::Template;
use axum::response::Html;

/// Renders a page. A template can't fail short of a bug; if one does, the
/// browser gets a plain message rather than half a page.
pub(crate) fn render(page: &impl Template) -> String {
    page.render().unwrap_or_else(|_| {
        "<!doctype html><title>Hoista</title><p>This page couldn't be shown.</p>".to_owned()
    })
}

/// [`render`] as a response body.
pub(crate) fn html(page: &impl Template) -> Html<String> {
    Html(render(page))
}

/// A page that only says how something ended, with Hoista's tile.
#[must_use]
pub fn result_page(title: &str, text: &str) -> String {
    render(&ResultPage {
        title,
        text,
        left: None,
        done: false,
    })
}

/// The page a provider's sign-in comes back to: the provider's logo (or its
/// first letter) beside Hoista's, ticked when it connected.
#[must_use]
pub fn provider_result(
    provider: &str,
    logo: Option<&str>,
    connected: bool,
    title: &str,
    text: &str,
) -> String {
    let letter;
    let left = if let Some(logo) = logo {
        logo
    } else {
        letter = render(&Letter {
            letter: provider.chars().next().unwrap_or('H'),
        });
        letter.as_str()
    };
    render(&ResultPage {
        title,
        text,
        left: Some(left),
        done: connected,
    })
}

/// [`provider_result`] for the use cases, which only say how a sign-in ended.
pub struct ProviderSignInPage;

impl crate::application::ports::sign_in_page::SignInPage for ProviderSignInPage {
    fn render(
        &self,
        provider: &str,
        logo: &str,
        connected: bool,
        title: &str,
        text: &str,
    ) -> String {
        provider_result(provider, Some(logo), connected, title, text)
    }
}

#[derive(Template)]
#[template(source = r#"<b class="letter">{{ letter }}</b>"#, ext = "html")]
struct Letter {
    letter: char,
}

#[derive(Template)]
#[template(path = "result.html")]
pub(crate) struct ResultPage<'a> {
    pub title: &'a str,
    pub text: &'a str,
    /// The drawing of what connected to Hoista, if anything did.
    pub left: Option<&'a str>,
    pub done: bool,
}

/// Words in a [`MessagePage`] sentence.
pub(crate) enum Inline<'a> {
    Text(&'a str),
    Code(&'a str),
    /// A label and where it goes.
    Link(&'a str, &'a str),
}

/// Hoista's tile and one card: a heading, a sentence, and maybe a button.
#[derive(Template)]
#[template(path = "message.html")]
pub(crate) struct MessagePage<'a> {
    pub title: &'a str,
    pub heading: &'a str,
    pub text: &'a [Inline<'a>],
    /// A label and where it goes.
    pub button: Option<(&'a str, &'a str)>,
}

/// One sentence per access level, the way the consent page sums it up, as
/// markup: the agent's name is in bold.
#[derive(Template)]
#[template(
    source = "{% match level %}{% when 0 %}This will let <b>{{ name }}</b> see your projects.{% when 1 %}This will let <b>{{ name }}</b> see and change your projects. It asks you first before anything it can't undo.{% else %}This will let <b>{{ name }}</b> see and change your projects, including deleting things, without asking you first.{% endmatch %}",
    ext = "html"
)]
struct Summary<'a> {
    level: usize,
    name: &'a str,
}

/// [`Summary`], rendered.
pub(crate) fn summary(level: usize, name: &str) -> String {
    render(&Summary { level, name })
}

/// One of the choices in a `<select>`.
pub(crate) struct Choice<'a> {
    pub value: &'a str,
    pub label: &'a str,
    pub selected: bool,
}

/// How the consent page sums up one access level, for its script.
pub(crate) struct LevelSaid {
    /// Markup: the agent's name is in bold.
    pub summary: String,
    pub asks: String,
}

pub(crate) struct Permission<'a> {
    pub title: &'a str,
    pub detail: &'a str,
    pub scope: &'a str,
    /// The lowest level that has it.
    pub from: usize,
    pub hidden: bool,
}

pub(crate) struct EnvironmentChoice<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

pub(crate) struct ProviderMark<'a> {
    pub name: &'a str,
    /// Trusted markup.
    pub logo: &'a str,
}

pub(crate) struct ProjectRow<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// The name, lowercase, for the search box.
    pub find: String,
    pub environment_id: &'a str,
    /// Its environment's name, shown when there are several.
    pub environment: Option<String>,
    pub providers: Vec<ProviderMark<'a>>,
}

/// A connected account and the projects that run on it.
pub(crate) struct AccountUse<'a> {
    /// Space-separated project ids.
    pub project_ids: String,
    /// Trusted markup.
    pub logo: &'a str,
    pub provider: &'a str,
    pub label: &'a str,
}

/// A ticked box with a bold title and a hint.
pub(crate) struct Tickbox {
    pub title: String,
    pub hint: String,
}

/// An agent asks to use Hoista.
#[derive(Template)]
#[template(path = "consent.html")]
pub(crate) struct ConsentPage<'a> {
    pub title: String,
    pub place: &'a str,
    pub returns_to: Option<&'a str>,
    pub token: String,
    pub back: Option<&'a str>,
    pub name: &'a str,
    pub code: &'a str,
    pub levels_said: Vec<LevelSaid>,
    /// Markup, like [`LevelSaid::summary`].
    pub summary_first: String,
    pub asks_first: String,
    pub permissions: Vec<Permission<'a>>,
    pub can_ask: bool,
    pub chosen: String,
    pub environments: Vec<EnvironmentChoice<'a>>,
    pub projects: Vec<ProjectRow<'a>>,
    pub uses: Vec<AccountUse<'a>>,
    pub levels: Vec<Choice<'a>>,
    pub ends: Vec<Choice<'a>>,
    pub replace: Option<Tickbox>,
    pub setup: Option<Tickbox>,
    /// Reload once the request is answered elsewhere.
    pub follow: bool,
}

/// After an agent was allowed.
#[derive(Template)]
#[template(path = "connected.html")]
pub(crate) struct ConnectedPage<'a> {
    pub title: &'a str,
    pub name: &'a str,
    pub environments: &'a str,
    pub projects: &'a str,
    pub level: &'a str,
    pub ends: &'a str,
    /// Whether setting the tool up worked, when it was asked for.
    pub set_up: Option<bool>,
    pub back: Option<&'a str>,
}

/// An agent's sign-in opened in a browser. With `review`, on a server: sign in
/// and answer there. Without, on a computer: answer on that computer while
/// this page waits.
#[derive(Template)]
#[template(path = "agent_waiting.html")]
pub(crate) struct AgentWaitingPage<'a> {
    pub title: &'a str,
    pub code: &'a str,
    pub review: Option<&'a str>,
}

/// Setting up a server: the claim code and a first passkey.
#[derive(Template)]
#[template(path = "owner/claim.html")]
pub(crate) struct ClaimPage<'a> {
    pub title: &'a str,
    pub next: &'a str,
    pub for_app: bool,
}

/// Signing in to a server with a passkey.
#[derive(Template)]
#[template(path = "owner/sign_in.html")]
pub(crate) struct SignInPage<'a> {
    pub title: &'a str,
    pub next: &'a str,
    pub for_app: bool,
}

pub(crate) struct DeviceRow<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// Like `6 Nov 2026`.
    pub added: String,
}

pub(crate) struct PasskeyRow<'a> {
    pub id: &'a str,
    pub name: &'a str,
}

/// A server's own page.
#[derive(Template)]
#[template(path = "owner/home.html")]
pub(crate) struct OwnerHomePage<'a> {
    pub title: &'a str,
    pub next: &'a str,
    pub host: &'a str,
    pub origin: &'a str,
    pub devices: Vec<DeviceRow<'a>>,
    pub passkeys: Vec<PasskeyRow<'a>>,
    pub version: &'a str,
    /// The image digest the server runs, when known.
    pub image: Option<&'a str>,
}

/// Allowing a computer by the code on its screen.
#[derive(Template)]
#[template(path = "devices/allow.html")]
pub(crate) struct AllowComputerPage<'a> {
    pub title: &'a str,
}

/// The Hoista app on a computer asks to manage this server.
#[derive(Template)]
#[template(path = "devices/connect.html")]
pub(crate) struct ConnectAppPage<'a> {
    pub title: &'a str,
    pub name: &'a str,
    pub host: &'a str,
    /// Where Cancel goes.
    pub deny: &'a str,
    /// JSON the page posts back when allowed.
    pub fields: String,
}

/// `2026-11-06…` as `6 Nov 2026`; anything else as it is.
#[must_use]
pub(crate) fn day(date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let parts: Vec<&str> = date.get(..10).unwrap_or(date).split('-').collect();
    match parts.as_slice() {
        [year, month, dd] => match (month.parse::<usize>(), dd.parse::<u32>()) {
            (Ok(m @ 1..=12), Ok(d)) => format!("{d} {} {year}", MONTHS[m - 1]),
            _ => date.to_owned(),
        },
        _ => date.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_drawings_are_not() {
        let page = provider_result("Railway", None, true, "Railway is connected", "a<b \"c\"");
        assert!(!page.contains("a<b"));
        assert!(page.contains(r#"<b class="letter">R</b>"#));
        assert!(page.contains(r#"<span class="badge">"#));
        let page = provider_result("X", Some("<svg id=\"x\"></svg>"), false, "t", "u");
        assert!(page.contains("<svg id=\"x\"></svg>"));
        assert!(!page.contains(r#"<span class="badge">"#));
    }

    #[test]
    fn every_page_has_the_shell() {
        let page = result_page("Hoista is running", "x");
        assert!(page.starts_with("<!doctype html>"));
        assert!(page.contains("<title>Hoista is running · Hoista</title>"));
        assert!(page.contains("--bg:#F4F4F5"));
        assert!(page.contains(r#"<span class="tile hoista"><svg"#));
    }

    #[test]
    fn days_read_as_people_write_them() {
        assert_eq!(day("2026-11-06T10:00:00Z"), "6 Nov 2026");
        assert_eq!(day("soon"), "soon");
    }
}

#[cfg(test)]
mod consent_tests {
    use super::*;

    fn sample() -> String {
        render(&ConsentPage {
            title: "Authorize claude-code".into(),
            place: "Hoista on this computer",
            returns_to: None,
            token: "t0k".into(),
            back: None,
            name: "Claude <Code>",
            code: "ABCD-1234",
            levels_said: (0..3)
                .map(|level| LevelSaid {
                    summary: summary(level, "Claude <Code>"),
                    asks: "asks".into(),
                })
                .collect(),
            summary_first: summary(1, "Claude <Code>"),
            asks_first: "Deleting or removing anything asks you first.".into(),
            permissions: vec![Permission {
                title: "See your projects",
                detail: "their status",
                scope: "apps:read",
                from: 0,
                hidden: false,
            }],
            can_ask: false,
            chosen: "Ask for approval · Every environment".into(),
            environments: vec![EnvironmentChoice {
                id: "env_default",
                name: "Production",
            }],
            projects: vec![ProjectRow {
                id: "p1",
                name: "Shop",
                find: "shop".into(),
                environment_id: "env_default",
                environment: Some("Production".into()),
                providers: vec![ProviderMark {
                    name: "Railway",
                    logo: "<svg></svg>",
                }],
            }],
            uses: vec![],
            levels: vec![Choice {
                value: "view",
                label: "View only",
                selected: true,
            }],
            ends: vec![Choice {
                value: "0",
                label: "No end date",
                selected: true,
            }],
            replace: Some(Tickbox {
                title: "Replace the earlier Claude Code connection".into(),
                hint: "Connected before".into(),
            }),
            setup: None,
            follow: true,
        })
    }

    #[test]
    fn the_consent_page_escapes_the_agent_and_keeps_its_script() {
        let page = sample();
        assert!(page.contains(r#"id="consent""#));
        assert!(!page.contains("<Code>"));
        assert!(page.contains("var at={view:0,ask:1,full:2}"));
        assert!(page.contains("setInterval("));
        assert!(page.contains(r#"<span title="Railway"><svg></svg></span>"#));
        assert!(page.contains("can't show Hoista's questions") || page.contains("can&#39;t show"));
    }
}
