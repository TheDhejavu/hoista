use url::Url;

const LOOPBACK: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];
/// The port a Hoista server listens on in Docker unless told otherwise.
const DEFAULT_SERVER_PORT: u16 = 8787;

/// A remote's address as `https://host`, or plain HTTP for one in Docker on
/// this computer. Anything with a path, query or credentials is refused.
pub fn origin(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    let loopback = url.host_str().is_some_and(|host| LOOPBACK.contains(&host));
    let secure = url.scheme() == "https" || (url.scheme() == "http" && loopback);
    let bare = url.path() == "/"
        && url.query().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    (secure && bare).then(|| url.origin().ascii_serialization())
}

/// What the person typed, as an address: `localhost:8787` means HTTP,
/// anything else without a scheme means HTTPS.
pub fn from_typed(typed: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.contains("://") {
        return origin(typed);
    }
    let host = typed.split([':', '/']).next().unwrap_or_default();
    let scheme = if LOOPBACK.contains(&host) || typed.starts_with("[::1]") {
        "http"
    } else {
        "https"
    };
    origin(&format!("{scheme}://{typed}"))
}

/// A name for a remote from its address: the first part of its host name,
/// or "Local Docker" for one on this computer.
pub fn default_name(origin: &str) -> String {
    let Ok(url) = Url::parse(origin) else {
        return "Remote".to_owned();
    };
    let host = url.host_str().unwrap_or_default();
    if LOOPBACK.contains(&host) {
        return match url.port() {
            Some(port) if port != DEFAULT_SERVER_PORT => format!("Local Docker ({port})"),
            _ => "Local Docker".to_owned(),
        };
    }
    host.split('.')
        .next()
        .unwrap_or(host)
        .chars()
        .take(super::MAX_NAME_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_secure_bare_addresses() {
        assert_eq!(
            origin("https://hoista.example.com/").as_deref(),
            Some("https://hoista.example.com")
        );
        assert_eq!(
            origin("http://localhost:8787").as_deref(),
            Some("http://localhost:8787")
        );
        assert_eq!(origin("http://hoista.example.com"), None);
        assert_eq!(origin("https://hoista.example.com/api"), None);
        assert_eq!(origin("https://me:pw@hoista.example.com"), None);
        assert_eq!(origin("https://hoista.example.com/?a=1"), None);
    }

    #[test]
    fn reads_what_was_typed() {
        assert_eq!(
            from_typed("hoista.example.com").as_deref(),
            Some("https://hoista.example.com")
        );
        assert_eq!(
            from_typed("localhost:8787").as_deref(),
            Some("http://localhost:8787")
        );
        assert_eq!(
            from_typed(" https://h.example.com ").as_deref(),
            Some("https://h.example.com")
        );
    }

    #[test]
    fn names_a_remote_from_its_host() {
        assert_eq!(
            default_name("https://home-server.example.com"),
            "home-server"
        );
        assert_eq!(default_name("http://localhost:8787"), "Local Docker");
        assert_eq!(default_name("http://localhost:9000"), "Local Docker (9000)");
    }
}
