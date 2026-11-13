//! HTTP to providers' own API addresses only, with a size limit on answers,
//! and retries only where repeating a request is harmless.

use crate::{Provider, application::ports::catalog::EngineError, domain::Secret};
use reqwest::{Client, Method, StatusCode, Url};
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

/// Names the vault key on a request the credential keeper signs in; never sent to a provider.
const KEPT: &str = "x-hoista-credential";

/// The response headers Hoista reads from a provider: the body's type, when to
/// retry, gRPC's status, and the request IDs kept as evidence of a failure.
/// The credential keeper passes back only these.
pub const RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "retry-after",
    "grpc-status",
    "grpc-message",
    "cf-ray",
    "x-request-id",
];

/// The HTTP client every provider adapter shares: allow-listed API hosts, a
/// size limit on answers, and a cap on calls in flight.
#[derive(Clone)]
pub struct ProviderHttp {
    client: Client,
    /// Where provider tokens are handles (on a server), the credential keeper
    /// puts the real token on each request.
    keeper: Option<Arc<dyn crate::application::ports::keeper::CredentialKeeper>>,
    /// Where providers' command-line tools are kept, once downloaded.
    tools: Option<Arc<std::path::PathBuf>>,
    history: Arc<crate::network_history::NetworkHistory>,
    calls: Arc<Semaphore>,
    #[cfg(test)]
    test_base: Option<Url>,
    /// Providers requests were addressed to before going to `test_base`.
    #[cfg(test)]
    destinations: Arc<std::sync::Mutex<Vec<String>>>,
}

/// How long a provider may take before the call is noted in the log.
const SLOW: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_secs(45);
/// The most of a provider's error text that's shown.
const MAX_ERROR_CHARS: usize = 240;
/// The most of a request path that's logged.
const MAX_LOGGED_PATH_CHARS: usize = 160;
/// The most of a refusal's body that's read for its explanation.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
/// The most a response body, or a log stream window, may hold.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Retries for a refused or failed read; a `retry-after` longer than
/// `MAX_RETRY_AFTER_SECS` isn't waited out.
const MAX_RETRIES: u32 = 3;
const MAX_RETRY_AFTER_SECS: u64 = 5;
const FIRST_BACKOFF_MS: u64 = 250;
const LONGEST_BACKOFF_MS: u64 = 10_000;
/// How many of a provider's error messages are passed on.
const MAX_ERRORS_SHOWN: usize = 3;

impl ProviderHttp {
    #[cfg(test)]
    pub(crate) fn local_fixture(base: Url) -> Self {
        assert_eq!(base.host_str(), Some("127.0.0.1"));
        Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .expect("fixture client"),
            history: Arc::new(crate::network_history::NetworkHistory::default()),
            calls: Arc::new(Semaphore::new(1)),
            keeper: None,
            tools: None,
            test_base: Some(base),
            destinations: Arc::default(),
        }
    }

    /// The client: HTTPS only, no redirects, and no proxy from the environment.
    ///
    /// # Errors
    /// Fails when TLS or the client can't be set up.
    pub fn new() -> Result<Self, EngineError> {
        let client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .user_agent(concat!("hoista/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| transport())?;
        Ok(Self {
            client,
            history: crate::network_history::shared(),
            calls: Arc::new(Semaphore::new(16)),
            keeper: None,
            tools: None,
            #[cfg(test)]
            test_base: None,
            #[cfg(test)]
            destinations: Arc::default(),
        })
    }

    #[cfg(test)]
    fn addressed(&self, url: &Url) {
        if let Some(host) = url.host_str() {
            self.destinations.lock().unwrap().push(host.to_owned());
        }
    }

    /// In tests, the same path on the local fixture provider, noting which
    /// provider it was meant for. Elsewhere, `url` unchanged.
    #[cfg(test)]
    fn local_in_tests(&self, url: Url) -> Url {
        self.addressed(&url);
        let Some(base) = &self.test_base else {
            return url;
        };
        let mut local = base.clone();
        local.set_path(url.path());
        local
    }

    #[cfg(not(test))]
    fn local_in_tests(&self, url: Url) -> Url {
        url
    }

    /// Whether this is the conformance suite's local provider.
    #[cfg(test)]
    pub(crate) fn is_fixture(&self) -> bool {
        self.test_base.is_some()
    }

    /// The providers requests were addressed to since the last call, for the
    /// conformance suite.
    #[cfg(test)]
    pub(crate) fn take_destinations(&self) -> Vec<String> {
        std::mem::take(&mut *self.destinations.lock().unwrap())
    }

    /// Provider requests carry handles, and the credential keeper adds the tokens.
    #[must_use]
    pub fn with_keeper(
        mut self,
        keeper: Arc<dyn crate::application::ports::keeper::CredentialKeeper>,
    ) -> Self {
        self.keeper = Some(keeper);
        self
    }

    /// Keeps providers' command-line tools in `dir`.
    #[must_use]
    pub fn with_tools(mut self, dir: std::path::PathBuf) -> Self {
        self.tools = Some(Arc::new(dir));
        self
    }

    /// Where providers' command-line tools are kept.
    pub(super) fn tools(&self) -> Option<&std::path::Path> {
        self.tools.as_deref().map(std::path::PathBuf::as_path)
    }

    /// The keeper, for the Fly.io CLI.
    pub(super) fn keeper(
        &self,
    ) -> Option<&Arc<dyn crate::application::ports::keeper::CredentialKeeper>> {
        self.keeper.as_ref()
    }

    /// Signs a request in the way `provider` takes its token, or, where the
    /// token is a handle (on a server), names it for the keeper to sign.
    fn authorize(
        request: reqwest::RequestBuilder,
        provider: Provider,
        token: &Secret,
    ) -> Result<reqwest::RequestBuilder, EngineError> {
        if let Some(key) = token.handle_key() {
            return Ok(request.header(KEPT, key));
        }
        let mut request = request;
        for (name, value) in super::entry(provider).api.sign_in.headers(token)? {
            request = request.header(name, value);
        }
        Ok(request)
    }

    /// Sends one signed request to the provider's API and reads the JSON answer.
    /// Reads are retried, and so are rate-limited writes when the provider says
    /// the limit ends within seconds.
    ///
    /// # Errors
    /// Authentication, permission, rate-limit, timeout and unreadable-answer
    /// errors, worded so they never echo the provider's private details.
    pub async fn json(
        &self,
        provider: Provider,
        token: &Secret,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, EngineError> {
        let url = self.url(provider, path, query)?;
        let request = Self::authorize(self.client.request(method.clone(), url), provider, token)?;
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        let result = self
            .execute_raw(request, method == Method::GET || method == Method::HEAD)
            .await;
        vet(
            result,
            super::entry(provider)
                .api
                .explains_refusals
                .then_some(token),
        )
    }

    /// Posts an OAuth token request (form-encoded, no bearer token) to a fixed
    /// endpoint on a published API host. Never retried: an authorization code
    /// works once, and a rotated refresh token must not be sent twice.
    pub(super) async fn oauth_form(
        &self,
        endpoint: &str,
        form: &[(&str, &str)],
    ) -> Result<Value, EngineError> {
        let url = Url::parse(endpoint).map_err(|_| transport())?;
        if !url.host_str().is_some_and(super::allowed_host) || url.scheme() != "https" {
            return Err(transport());
        }
        let url = self.local_in_tests(url);
        self.execute(self.client.post(url).form(form), false).await
    }

    /// An absolute address on one of `provider`'s own API hosts (an upload
    /// or a second API beside its base), with path segments and query added.
    /// Any other host is refused, so a token only ever goes to its provider.
    pub(super) fn at(
        &self,
        provider: Provider,
        address: &str,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Url, EngineError> {
        let url = Url::parse(address).map_err(|_| transport())?;
        if url.scheme() != "https"
            || !url
                .host_str()
                .is_some_and(|host| super::entry(provider).api.hosts.contains(&host))
        {
            return Err(transport());
        }
        let mut url = self.local_in_tests(url);
        if !path.is_empty() {
            let mut segments = url.path_segments_mut().map_err(|_| transport())?;
            segments.pop_if_empty();
            for segment in path {
                segments.push(segment);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        Ok(url)
    }

    /// A request signed in with the provider token, for an adapter to finish
    /// (headers, body) and hand to [`Self::send_request`].
    pub(super) fn signed(
        &self,
        method: Method,
        provider: Provider,
        url: Url,
        token: &Secret,
    ) -> Result<reqwest::RequestBuilder, EngineError> {
        Self::authorize(self.client.request(method, url), provider, token)
    }

    /// Sends a request built with [`Self::signed`]. Reads (`retry_read`)
    /// are retried; anything else is sent once.
    pub(super) async fn send_request(
        &self,
        request: reqwest::RequestBuilder,
        retry_read: bool,
    ) -> Result<Value, EngineError> {
        self.execute(request, retry_read).await
    }

    /// Posts a JSON body to an absolute address on the provider's own API hosts.
    pub(super) async fn post_at(
        &self,
        provider: Provider,
        token: &Secret,
        address: &str,
        body: &Value,
        read: bool,
    ) -> Result<Value, EngineError> {
        let url = self.at(provider, address, &[], &[])?;
        self.execute(
            self.signed(Method::POST, provider, url, token)?.json(body),
            read,
        )
        .await
    }

    /// Reads JSON from an absolute address on the provider's own API hosts.
    /// Retried like any read.
    pub(super) async fn get_at(
        &self,
        provider: Provider,
        token: &Secret,
        address: &str,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Value, EngineError> {
        let url = self.at(provider, address, path, query)?;
        self.execute(self.signed(Method::GET, provider, url, token)?, true)
            .await
    }

    /// One gRPC call to an absolute address on the provider's own API hosts:
    /// `body` is the framed request, `headers` any extra headers the provider
    /// wants; the call is signed in the provider's own way. Returns the response
    /// body (its frames). A call is sent once. gRPC failures come back as
    /// `grpc-status` headers, mapped like HTTP failures.
    ///
    /// The conformance fixture speaks plain HTTP; there the call goes out as
    /// JSON to the test provider.
    pub(super) async fn grpc(
        &self,
        provider: Provider,
        token: &Secret,
        address: &str,
        headers: &[(&'static str, String)],
        body: Vec<u8>,
    ) -> Result<Vec<u8>, EngineError> {
        let url = self.at(provider, address, &[], &[])?;
        let mut request = if url.scheme() == "https" {
            let request = self
                .client
                .post(url)
                .version(reqwest::Version::HTTP_2)
                .header("content-type", "application/grpc")
                .header("te", "trailers");
            Self::authorize(request, provider, token)?
        } else {
            Self::authorize(self.client.post(url), provider, token)?
                .header("content-type", "application/json")
        };
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let _permit = self.calls.acquire().await.map_err(|_| transport())?;
        let mut response = self.send(request.body(body)).await?;
        if !response.status().is_success() {
            return vet(parse(response).await, None).map(|_| Vec::new());
        }
        if let Some(status) = response
            .headers()
            .get("grpc-status")
            .and_then(|value| value.to_str().ok())
            && status != "0"
        {
            let said = response
                .headers()
                .get("grpc-message")
                .and_then(|value| value.to_str().ok())
                .map(|text| {
                    percent_decoded(text)
                        .chars()
                        .take(MAX_ERROR_CHARS)
                        .collect::<String>()
                })
                .unwrap_or_default();
            let failed: Result<Value, EngineError> = Err(grpc_failure(status, &said));
            return vet(
                failed,
                super::entry(provider)
                    .api
                    .explains_refusals
                    .then_some(token),
            )
            .map(|_| Vec::new());
        }
        read_body(&mut response).await
    }

    /// Reads a server-sent event stream for a short window and returns each
    /// event's `data`: what a log stream backfills, then whatever arrives
    /// until `window` passes, `most` events have come, or the provider ends it.
    pub(super) async fn sse_window(
        &self,
        provider: Provider,
        token: &Secret,
        path: &[&str],
        query: &[(&str, &str)],
        most: usize,
        window: Duration,
    ) -> Result<Vec<String>, EngineError> {
        let url = self.url(provider, path, query)?;
        let request = self
            .signed(Method::GET, provider, url, token)?
            .header("accept", "text/event-stream");
        let _permit = self.calls.acquire().await.map_err(|_| transport())?;
        let mut response = self.send(request).await?;
        if !response.status().is_success() {
            return vet(
                parse(response).await,
                super::entry(provider)
                    .api
                    .explains_refusals
                    .then_some(token),
            )
            .map(|_| Vec::new());
        }
        let until = tokio::time::Instant::now() + window;
        let mut pending = String::new();
        let mut data: Vec<String> = Vec::new();
        let mut current: Vec<String> = Vec::new();
        let mut read = 0usize;
        while data.len() < most {
            let left = until.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let chunk = match tokio::time::timeout(left, response.chunk()).await {
                Ok(Ok(Some(chunk))) => chunk,
                Ok(Ok(None)) | Err(_) => break,
                Ok(Err(_)) => return Err(transport()),
            };
            read = read.saturating_add(chunk.len());
            if read > MAX_RESPONSE_BYTES {
                break;
            }
            pending.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = pending.find('\n') {
                let line: String = pending.drain(..=end).collect();
                let line = line.trim_end_matches(['\n', '\r']);
                if line.is_empty() {
                    if !current.is_empty() {
                        data.push(current.join("\n"));
                        current.clear();
                    }
                } else if let Some(value) = line.strip_prefix("data:") {
                    current.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
                }
            }
        }
        data.truncate(most);
        Ok(data)
    }

    /// Uploads a multipart body to one of the provider's API addresses.
    ///
    /// # Errors
    /// Upload failures. A failed upload is never sent again, since it may have
    /// gone through.
    pub async fn multipart(
        &self,
        provider: Provider,
        token: &Secret,
        method: Method,
        path: &[&str],
        query: &[(&str, &str)],
        form: reqwest::multipart::Form,
    ) -> Result<Value, EngineError> {
        let url = self.url(provider, path, query)?;
        self.execute(
            Self::authorize(self.client.request(method, url), provider, token)?.multipart(form),
            false,
        )
        .await
    }

    /// Checks a provider-assigned public URL without credentials or redirects.
    ///
    /// # Errors
    /// Rejects unsupported providers, non-HTTPS URLs and failed health checks.
    pub async fn health(&self, url: &str) -> Result<(), EngineError> {
        let url = Url::parse(url).map_err(|_| transport())?;
        let host = url.host_str().ok_or_else(transport)?;
        if url.scheme() != "https"
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || ![".workers.dev", ".fly.dev", ".vercel.app", ".pages.dev"]
                .iter()
                .any(|suffix| host.ends_with(suffix))
        {
            return Err(EngineError::Invalid(
                "Health checks require a provider-assigned HTTPS hostname.",
            ));
        }
        let _permit = self.calls.acquire().await.map_err(|_| transport())?;
        let status = self
            .send(self.client.get(url))
            .await
            .map_err(|_| transport())?
            .status();
        if !status.is_success() {
            return Err(EngineError::Provider {
                evidence: Some(crate::domain::model::ProviderFailureEvidence {
                    http_status: status.as_u16(),
                    request_id: None,
                    request_id_header: None,
                }),
                code: "deploy.health_failed".into(),
                message: "The public endpoint did not return a successful response. Inspect the app logs before retrying.".into(),
            });
        }
        Ok(())
    }

    pub(super) fn url(
        &self,
        provider: Provider,
        path: &[&str],
        query: &[(&str, &str)],
    ) -> Result<Url, EngineError> {
        let base = super::entry(provider).api.base;
        let mut url = Url::parse(base).map_err(|_| transport())?;
        if !url.host_str().is_some_and(super::allowed_host) {
            return Err(transport());
        }

        #[cfg(test)]
        self.addressed(&url);
        #[cfg(test)]
        if let Some(base) = &self.test_base {
            url = base.clone();
        }
        {
            let mut segments = url.path_segments_mut().map_err(|_| transport())?;
            segments.pop_if_empty();
            for segment in path {
                segments.push(segment);
            }
        }
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().copied());
        }
        Ok(url)
    }

    async fn execute(
        &self,
        request: reqwest::RequestBuilder,
        retry_read: bool,
    ) -> Result<Value, EngineError> {
        vet(self.execute_raw(request, retry_read).await, None)
    }

    async fn execute_raw(
        &self,
        request: reqwest::RequestBuilder,
        retry_read: bool,
    ) -> Result<Value, EngineError> {
        for attempt in 0..=MAX_RETRIES {
            // Each attempt takes its own turn, so a call waiting to retry
            // doesn't hold up everyone else's.
            let permit = self.calls.acquire().await.map_err(|_| transport())?;
            let next = request.try_clone();
            // A body that can't be cloned (a streamed multipart) is sent once.
            let builder = match next {
                Some(next) => next,
                None => return self.once(request).await,
            };
            let response = self.send(builder).await;
            let retry = match &response {
                // A rate-limited write (or GraphQL POST) is retried only when the
                // provider says the limit is over in a few seconds; otherwise
                // retrying spends more of the allowance.
                Ok(response) if response.status() == StatusCode::TOO_MANY_REQUESTS => {
                    retry_read
                        || response
                            .headers()
                            .get("retry-after")
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.parse::<u64>().ok())
                            .is_some_and(|seconds| seconds <= MAX_RETRY_AFTER_SECS)
                }
                Ok(response) => retry_read && response.status().is_server_error(),
                Err(EngineError::Provider { code, .. }) => {
                    retry_read && (code == "provider.timeout" || code == "provider.unreachable")
                }
                Err(_) => false,
            };
            if retry && attempt < MAX_RETRIES {
                let retry_after = response
                    .as_ref()
                    .ok()
                    .and_then(|r| r.headers().get("retry-after"))
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                let millis = retry_after
                    .map(|seconds| seconds.saturating_mul(1000))
                    .unwrap_or_else(|| {
                        let jitter = u64::from(uuid::Uuid::new_v4().as_bytes()[0]);
                        FIRST_BACKOFF_MS * (1_u64 << attempt) + jitter
                    })
                    .min(LONGEST_BACKOFF_MS);
                drop(permit);
                #[cfg(not(test))]
                tokio::time::sleep(Duration::from_millis(millis)).await;
                #[cfg(test)]
                let _ = millis;
                continue;
            }
            return parse(response?).await;
        }
        Err(transport())
    }

    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, EngineError> {
        let mut request = builder.build().map_err(|_| transport())?;
        let attempt = self.history.begin(request.url());
        let started = std::time::Instant::now();
        // Which provider and endpoint, for the log: never the query, where
        // anything sensitive could be.
        let host = request.url().host_str().unwrap_or_default().to_owned();
        let path = request
            .url()
            .path()
            .chars()
            .take(MAX_LOGGED_PATH_CHARS)
            .collect::<String>();
        let response = match request.headers_mut().remove(KEPT) {
            Some(key) => {
                self.kept(&request, key.to_str().map_err(|_| transport())?)
                    .await
            }
            None => self.client.execute(request).await.map_err(|error| {
                if error.is_timeout() {
                    EngineError::Provider {
                        evidence: None,
                        code: "provider.timeout".into(),
                        message: "The provider didn't answer in time.".into(),
                    }
                } else {
                    transport()
                }
            }),
        };
        attempt.finish(
            response
                .as_ref()
                .ok()
                .map(|response| response.status().as_u16()),
        );
        let took = started.elapsed();
        if took >= SLOW {
            tracing::info!(
                code = "provider.slow",
                host = host.as_str(),
                path = path.as_str(),
                ms = u64::try_from(took.as_millis()).unwrap_or(u64::MAX),
                "A provider answered slowly"
            );
        }
        response
    }

    /// Sends a request through the credential keeper, which adds the token.
    async fn kept(
        &self,
        request: &reqwest::Request,
        key: &str,
    ) -> Result<reqwest::Response, EngineError> {
        let keeper = self.keeper.as_ref().ok_or(EngineError::Store)?;
        let body = match request.body() {
            None => Vec::new(),
            Some(body) => body
                .as_bytes()
                .ok_or(EngineError::Invalid(
                    "This upload can't go through the credential keeper yet.",
                ))?
                .to_vec(),
        };
        let headers = request
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.as_str().to_owned(), value.to_owned()))
            })
            .collect();
        let answer = keeper
            .http(
                key,
                request.method().as_str(),
                request.url().as_str(),
                headers,
                &body,
                request.version() == reqwest::Version::HTTP_2,
            )
            .await?;
        let mut response = http::Response::builder().status(answer.status);
        for (name, value) in answer.headers {
            response = response.header(name, value);
        }
        Ok(reqwest::Response::from(
            response.body(answer.body).map_err(|_| transport())?,
        ))
    }

    /// Posts a GraphQL body and returns the provider's whole answer, errors
    /// included, so a caller can keep the data a partly refused read still
    /// returns and see which fields were refused. Not retried; HTTP failures
    /// other than a GraphQL 400 are errors as usual.
    pub(super) async fn graphql_raw(
        &self,
        provider: Provider,
        token: &Secret,
        body: &Value,
    ) -> Result<Value, EngineError> {
        let url = self.url(provider, &[], &[])?;
        let request =
            Self::authorize(self.client.request(Method::POST, url), provider, token)?.json(body);
        let _permit = self.calls.acquire().await.map_err(|_| transport())?;
        let mut response = self.send(request).await?;
        let status = response.status().as_u16();
        if status != 200 && status != 400 {
            return vet(parse(response).await, None);
        }
        let bytes = read_body(&mut response).await?;
        serde_json::from_slice(&bytes).map_err(|_| EngineError::Provider {
            evidence: None,
            code: "provider.invalid_response".into(),
            message: "The provider returned an unreadable response. Try again later.".into(),
        })
    }

    async fn once(&self, request: reqwest::RequestBuilder) -> Result<Value, EngineError> {
        parse(self.send(request).await?).await
    }
}

/// The whole body, refused past `MAX_RESPONSE_BYTES`.
async fn read_body(response: &mut reqwest::Response) -> Result<Vec<u8>, EngineError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| transport())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(EngineError::Provider {
                evidence: None,
                code: "provider.response_limit".into(),
                message: "The provider response was too large. Use a narrower query.".into(),
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Some providers explain refusals in words meant for their users (their
/// `ProviderApi::explains_refusals`); those are passed on with the token
/// redacted. Other providers' messages may echo private details, so they get the
/// generic sentence.
fn vet(result: Result<Value, EngineError>, explain: Option<&Secret>) -> Result<Value, EngineError> {
    match result {
        Err(EngineError::Provider {
            evidence,
            code,
            message,
        }) if code == "provider.rejected" || message.starts_with(EXPLAINED) => {
            let message = match (message.strip_prefix(EXPLAINED), explain) {
                (Some(said), Some(token)) => {
                    let scanner =
                        crate::domain::leaks::LeakScanner::new().map_err(|_| EngineError::Store)?;
                    format!(
                        "The provider rejected the operation: {}",
                        scanner.redact(said, std::slice::from_ref(token))
                    )
                }
                _ if code == "provider.token_scope" => {
                    "The token cannot perform this operation. Check the required provider scopes."
                        .to_owned()
                }
                _ => REJECTED.to_owned(),
            };
            Err(EngineError::Provider {
                evidence,
                code,
                message,
            })
        }
        other => other,
    }
}

const REJECTED: &str = "The provider rejected the operation. Check the account permissions and resource configuration.";
/// Marks a rejection that carries the provider's own words, for `json` to vet.
const EXPLAINED: &str = "\u{1}explained:";

fn transport() -> EngineError {
    EngineError::Provider {
        evidence: None,
        code: "provider.unreachable".into(),
        message: "The provider could not be reached. Check your connection and retry; inspect the provider before retrying a failed write.".into(),
    }
}

async fn parse(response: reqwest::Response) -> Result<Value, EngineError> {
    let evidence = response_evidence(response.status().as_u16(), response.headers());
    // Which provider and endpoint refused, for the log: the address without its
    // query, which is where anything sensitive could be.
    let host = response.url().host_str().unwrap_or_default().to_owned();
    let path = response
        .url()
        .path()
        .chars()
        .take(MAX_LOGGED_PATH_CHARS)
        .collect::<String>();
    parse_body(response).await.map_err(|error| {
        tracing::warn!(
            code = error.code(),
            host = host.as_str(),
            path = path.as_str(),
            http_status = evidence.http_status,
            request_id = evidence.request_id.as_deref(),
            request_id_header = evidence.request_id_header.as_deref(),
            "Provider response failed"
        );
        error.with_provider_response(evidence)
    })
}

fn response_evidence(
    status: u16,
    headers: &reqwest::header::HeaderMap,
) -> crate::domain::model::ProviderFailureEvidence {
    // Only narrowly shaped identifiers are retained, never arbitrary header strings.
    let reference = ["cf-ray", "x-request-id"].into_iter().find_map(|name| {
        let mut values = headers.get_all(name).iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() {
            return None;
        }
        let valid = match name {
            "cf-ray" => value.split_once('-').is_some_and(|(id, colo)| {
                id.len() == 16
                    && id.bytes().all(|b| b.is_ascii_hexdigit())
                    && colo.len() == 3
                    && colo.bytes().all(|b| b.is_ascii_uppercase())
            }),
            _ => value.len() == 36 && uuid::Uuid::parse_str(value).is_ok(),
        };
        valid.then(|| (name.to_owned(), value.to_owned()))
    });
    crate::domain::model::ProviderFailureEvidence {
        http_status: status,
        request_id: reference.as_ref().map(|(_, value)| value.clone()),
        request_id_header: reference.map(|(name, _)| name),
    }
}

/// A refusal in the provider's own words, short: GraphQL's `errors[].message`,
/// a problem document's `detail` with its field `errors` (Runpod), a REST
/// provider's `{"message": …}`, Fly's `{"error": …}`, or Cloudflare's
/// `errors[]` with the more exact reasons in each one's `error_chain`.
fn refusal_words(value: &Value) -> Option<String> {
    let short = |text: &str| text.chars().take(MAX_ERROR_CHARS).collect::<String>();
    let listed: Vec<String> = value
        .get("errors")
        .and_then(Value::as_array)
        .map(|errors| {
            errors
                .iter()
                .take(MAX_ERRORS_SHOWN)
                .filter_map(|error| {
                    let said = error
                        .as_str()
                        .or_else(|| error.get("message").and_then(Value::as_str))?;
                    let reasons: Vec<&str> = error
                        .get("error_chain")
                        .and_then(Value::as_array)
                        .map(|chain| {
                            chain
                                .iter()
                                .filter_map(|link| link.get("message").and_then(Value::as_str))
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(if reasons.is_empty() {
                        said.to_owned()
                    } else {
                        format!("{said} ({})", reasons.join("; "))
                    })
                })
                .filter(|text| !text.is_empty())
                .map(|text| short(&text))
                .collect()
        })
        .unwrap_or_default();
    let sentence = ["detail", "message", "error"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|text| !text.is_empty())
        .map(short);
    let said = match (sentence, listed.is_empty()) {
        (Some(sentence), true) => sentence,
        (Some(sentence), false) => format!("{sentence}: {}", listed.join("; ")),
        (None, false) => listed.join("; "),
        (None, true) => return None,
    };
    Some(said)
}

async fn parse_body(mut response: reqwest::Response) -> Result<Value, EngineError> {
    let status = response.status();
    if !status.is_success() {
        let (code, message) = match status.as_u16() {
            401 => (
                "provider.token_expired",
                "The provider token is invalid or expired. Reconnect the account.",
            ),
            403 => (
                "provider.token_scope",
                "The token cannot perform this operation. Check the required provider scopes.",
            ),
            404 => (
                "provider.not_found",
                "The provider resource was not found. Refresh the account resources.",
            ),
            429 => (
                "provider.rate_limited",
                "The provider is rate limiting requests. Wait and try again.",
            ),
            500..=599 => (
                "provider.unavailable",
                "The provider is unavailable. Inspect the last release before retrying a write.",
            ),
            _ => (
                "provider.rejected",
                "The provider rejected this operation. Check the resource configuration and account permissions.",
            ),
        };
        tracing::warn!(code, status = status.as_u16());
        // Keep a refusal's own words for `vet`, which shows them only for providers
        // that explain refusals (GraphQL providers answer a query they can't run
        // with 400 and their reasons). A 403 stays a token-scope error,
        // whatever reason the provider gives.
        let explained = if status.as_u16() == 403 {
            "provider.token_scope"
        } else {
            "provider.rejected"
        };
        if matches!(status.as_u16(), 400 | 402 | 403 | 410 | 422) {
            let mut bytes = Vec::new();
            while let Ok(Some(chunk)) = response.chunk().await {
                if bytes.len() + chunk.len() > MAX_ERROR_BODY_BYTES {
                    break;
                }
                bytes.extend_from_slice(&chunk);
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes)
                && let Some(said) = refusal_words(&value)
            {
                return Err(EngineError::Provider {
                    evidence: None,
                    code: explained.into(),
                    message: format!("{EXPLAINED}{said}"),
                });
            }
        }
        return Err(EngineError::Provider {
            evidence: None,
            code: code.into(),
            message: message.into(),
        });
    }
    let bytes = read_body(&mut response).await?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    // JSON, or JSON Lines (one object per line, as Vercel's billing charges
    // stream), read as a list.
    let value: Value = serde_json::from_slice(&bytes)
        .ok()
        .or_else(|| json_lines(&bytes))
        .ok_or_else(|| EngineError::Provider {
            evidence: None,
            code: "provider.invalid_response".into(),
            message: "The provider returned an unreadable response. Try again later.".into(),
        })?;
    if value.get("success") == Some(&Value::Bool(false))
        || value
            .get("errors")
            .is_some_and(|v| v.as_array().is_some_and(|a| !a.is_empty()))
    {
        let mut message = REJECTED.to_owned();
        // GraphQL providers (no `success` envelope) explain a refusal in words
        // meant for their users, such as "Deployment is not building". Keep
        // the first few, short; `vet` shows them only for providers that explain
        // refusals, redacted.
        if value.get("success").is_none()
            && let Some(errors) = value.get("errors").and_then(Value::as_array)
        {
            let said: Vec<String> = errors
                .iter()
                .take(MAX_ERRORS_SHOWN)
                .filter_map(|error| error.get("message").and_then(Value::as_str))
                .map(|text| text.chars().take(MAX_ERROR_CHARS).collect::<String>())
                .collect();
            if !said.is_empty() {
                message = format!("{EXPLAINED}{}", said.join("; "));
            }
        }
        return Err(EngineError::Provider {
            evidence: None,
            code: "provider.rejected".into(),
            message,
        });
    }
    Ok(value)
}

/// `%20`-style escapes in a gRPC message decoded.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(byte) = u8::from_str_radix(&text[index + 1..index + 3], 16)
        {
            out.push(byte);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A gRPC status as the engine's provider errors, the way HTTP statuses map.
fn grpc_failure(status: &str, said: &str) -> EngineError {
    let (code, message) = match status {
        "16" => (
            "provider.token_expired",
            "The provider token is invalid or expired. Reconnect the account.",
        ),
        "7" => (
            "provider.token_scope",
            "The token cannot perform this operation. Check the required provider scopes.",
        ),
        "5" => (
            "provider.not_found",
            "The provider resource was not found. Refresh the account resources.",
        ),
        "8" => (
            "provider.rate_limited",
            "The provider is rate limiting requests. Wait and try again.",
        ),
        "14" | "13" | "4" => (
            "provider.unavailable",
            "The provider is unavailable. Inspect the last release before retrying a write.",
        ),
        _ => (
            "provider.rejected",
            "The provider rejected this operation. Check the resource configuration and account permissions.",
        ),
    };
    tracing::warn!(code, grpc_status = status);
    // Invalid arguments and failed preconditions are explained in the provider's own words.
    if matches!(status, "3" | "9") && !said.is_empty() {
        return EngineError::Provider {
            evidence: None,
            code: "provider.rejected".into(),
            message: format!("{EXPLAINED}{said}"),
        };
    }
    EngineError::Provider {
        evidence: None,
        code: code.into(),
        message: message.into(),
    }
}

/// A bill from FOCUS rows (the FinOps billing format Cloudflare and Vercel
/// share): the sum of `field` across the rows, in their currency, from the
/// earliest period start. None when there are no rows.
pub(super) fn focus_bill(
    rows: &[Value],
    field: &str,
) -> Option<crate::application::ports::providers::ProviderBill> {
    if rows.is_empty() {
        return None;
    }
    let number = |value: &Value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
    };
    let total: f64 = rows.iter().filter_map(|row| number(&row[field])).sum();
    let currency = rows
        .iter()
        .find_map(|row| row["BillingCurrency"].as_str())
        .unwrap_or("USD")
        .to_owned();
    let start = rows
        .iter()
        .filter_map(|row| {
            row["BillingPeriodStart"]
                .as_str()
                .or_else(|| row["ChargePeriodStart"].as_str())
        })
        .min()
        .map(str::to_owned);
    let cents = crate::units::cents(total);
    Some(crate::application::ports::providers::ProviderBill {
        billed: crate::domain::model::Money { cents, currency },
        period_start: start,
        period_end: None,
        soft_limit: None,
        hard_limit: None,
        over_limit: false,
        customer_id: None,
    })
}

/// A body of JSON Lines as a list of its objects; None unless every
/// non-empty line is JSON.
fn json_lines(bytes: &[u8]) -> Option<Value> {
    let text = std::str::from_utf8(bytes).ok()?;
    let rows: Option<Vec<Value>> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).ok())
        .collect();
    rows.filter(|rows| rows.len() > 1 || text.trim().lines().count() == 1)
        .map(Value::Array)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn a_refusal_is_read_in_each_providers_shape() {
        let words = |body: Value| refusal_words(&body);
        // Runpod: a problem document with its field failures.
        assert_eq!(
            words(serde_json::json!({
                "title": "Bad Request",
                "status": 400,
                "detail": "Invalid request",
                "errors": ["cpu: required when gpu is not set"],
            })),
            Some("Invalid request: cpu: required when gpu is not set".to_owned())
        );
        // GraphQL.
        assert_eq!(
            words(serde_json::json!({"errors": [{"message": "Not Authorized"}]})),
            Some("Not Authorized".to_owned())
        );
        // REST and Fly.
        assert_eq!(
            words(serde_json::json!({"message": "No"})),
            Some("No".to_owned())
        );
        assert_eq!(
            words(serde_json::json!({"error": "app not found"})),
            Some("app not found".to_owned())
        );
        // Cloudflare's envelope, with the exact reason in its chain.
        assert_eq!(
            words(serde_json::json!({"success": false, "errors": [{
                "code": 6003,
                "message": "Invalid request headers",
                "error_chain": [{"code": 6111, "message": "Invalid format for Authorization header"}],
            }]})),
            Some("Invalid request headers (Invalid format for Authorization header)".to_owned())
        );
        // Nothing said.
        assert_eq!(words(serde_json::json!({"errors": []})), None);
    }

    async fn fixture(statuses: Vec<u16>) -> (ProviderHttp, tokio::task::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fixture listener");
        let address = listener.local_addr().expect("fixture address");
        let task = tokio::spawn(async move {
            let mut calls = 0;
            for status in statuses {
                let (mut stream, _) = listener.accept().await.expect("fixture request");
                let mut buffer = [0; 8192];
                let read = stream
                    .read(&mut buffer)
                    .await
                    .expect("fixture request body");
                assert!(!String::from_utf8_lossy(&buffer[..read]).contains("evil.example"));
                let body = if status == 200 {
                    r#"{"result":{"ok":true}}"#
                } else {
                    "sensitive-provider-detail"
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nCF-Ray: 0123456789abcdef-LHR\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("fixture response");
                calls += 1;
            }
            calls
        });
        (
            ProviderHttp {
                keeper: None,
                tools: None,
                client: Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_secs(2))
                    .build()
                    .expect("test client"),
                history: Arc::new(crate::network_history::NetworkHistory::default()),
                calls: Arc::new(Semaphore::new(1)),
                test_base: Some(Url::parse(&format!("http://{address}/")).expect("fixture URL")),
                destinations: Arc::default(),
            },
            task,
        )
    }

    #[tokio::test]
    async fn retries_reads_but_never_ambiguous_writes_and_never_echoes_bodies() {
        let token = Secret::new("private-token".into());
        let (http, task) = fixture(vec![429, 503, 200]).await;
        assert!(
            http.json(
                Provider::Cloudflare,
                &token,
                Method::GET,
                &["resources"],
                &[],
                None
            )
            .await
            .is_ok()
        );
        assert_eq!(task.await.unwrap(), 3);
        let history = http.history.snapshot();
        assert_eq!(history.recent_hosts.len(), 1);
        assert_eq!(history.recent_hosts[0].host, "127.0.0.1");
        assert_eq!(history.recent_hosts[0].attempts, 3);
        assert_eq!(history.recent_hosts[0].http_status, Some(200));
        let encoded = serde_json::to_string(&history).unwrap();
        for private in ["resources", "private-token", "sensitive-provider-detail"] {
            assert!(!encoded.contains(private));
        }
        for status in [401, 403, 503] {
            let (http, task) = fixture(vec![status]).await;
            let error = http
                .json(
                    Provider::Cloudflare,
                    &token,
                    Method::POST,
                    &["resources"],
                    &[],
                    Some(&serde_json::json!({})),
                )
                .await
                .unwrap_err();
            assert!(!error.to_string().contains("sensitive-provider-detail"));
            assert!(!format!("{error:?}").contains("private-token"));
            let evidence = error.provider_response().expect("response metadata");
            assert_eq!(evidence.http_status, status);
            assert_eq!(evidence.request_id.as_deref(), Some("0123456789abcdef-LHR"));
            // What clients are shown is built from these parts; the server's
            // contract tests cover how they are rendered.
            assert!(evidence.request_id_header.is_some());
            assert!(
                !serde_json::to_string(evidence)
                    .unwrap()
                    .contains("sensitive-provider-detail")
            );
            assert_eq!(task.await.unwrap(), 1);
        }
    }

    #[test]
    fn response_references_reject_arbitrary_duplicate_or_oversized_headers() {
        use reqwest::header::{HeaderMap, HeaderValue};
        for value in [
            "private-token",
            "0123456789abcdef-LHR/private",
            "0123456789abcdef-LH",
            "0123456789abcdef-lhr",
            "",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("cf-ray", HeaderValue::from_str(value).unwrap());
            headers.insert("x-request-id", HeaderValue::from_str(value).unwrap());
            assert!(response_evidence(403, &headers).request_id.is_none());
        }
        let mut headers = HeaderMap::new();
        headers.append("cf-ray", HeaderValue::from_static("0123456789abcdef-LHR"));
        headers.append("cf-ray", HeaderValue::from_static("fedcba9876543210-LHR"));
        assert!(response_evidence(403, &headers).request_id.is_none());
        headers.insert(
            "x-request-id",
            HeaderValue::from_static("550e8400-e29b-41d4-a716-446655440000"),
        );
        assert_eq!(
            response_evidence(403, &headers)
                .request_id_header
                .as_deref(),
            Some("x-request-id")
        );
        headers.insert(
            "x-request-id",
            HeaderValue::from_str(&"a".repeat(1024)).unwrap(),
        );
        assert!(response_evidence(403, &headers).request_id.is_none());
    }

    #[test]
    fn endpoint_segments_cannot_change_the_origin() {
        let http = ProviderHttp::new().unwrap();
        let url = http
            .url(
                Provider::Cloudflare,
                &["https://evil.example", "../tokens"],
                &[],
            )
            .unwrap();
        assert_eq!(url.host_str(), Some("api.cloudflare.com"));
        assert!(!url.path().contains("../"));
        let published = include_str!("../../../docs/backend/network-hosts.txt");
        assert_eq!(
            published.lines().collect::<Vec<_>>(),
            super::super::api_hosts()
        );
    }

    async fn response_fixture(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (ProviderHttp, tokio::task::JoinHandle<()>) {
        status_response_fixture(
            responses
                .into_iter()
                .map(|(path, body)| (path, 200, body))
                .collect(),
        )
        .await
    }

    async fn status_response_fixture(
        responses: Vec<(&'static str, u16, &'static str)>,
    ) -> (ProviderHttp, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("request");
                let mut buffer = vec![0; 65536];
                let size = stream.read(&mut buffer).await.expect("read");
                let request = String::from_utf8_lossy(&buffer[..size]);
                let (path, required_header) = path
                    .split_once('|')
                    .map_or((path, None), |(path, header)| (path, Some(header)));
                if let Some(header) = required_header {
                    assert!(
                        request
                            .lines()
                            .any(|line| line.to_ascii_lowercase() == header),
                        "missing expected request header"
                    );
                }
                assert!(
                    request.lines().next().expect("request line").contains(path),
                    "unexpected request path"
                );
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.expect("write");
            }
        });
        (
            ProviderHttp {
                keeper: None,
                tools: None,
                client: Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .expect("client"),
                history: Arc::new(crate::network_history::NetworkHistory::default()),
                calls: Arc::new(Semaphore::new(1)),
                test_base: Some(Url::parse(&format!("http://{address}/")).expect("URL")),
                destinations: Arc::default(),
            },
            task,
        )
    }

    /// Like `status_response_fixture`, and hands back each request as sent
    /// (request line, headers and body), to check what was asked for.
    async fn capturing_fixture(
        responses: Vec<(&'static str, u16, &'static str)>,
    ) -> (ProviderHttp, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let address = listener.local_addr().expect("address");
        let task = tokio::spawn(async move {
            let mut seen = Vec::new();
            for (path, status, body) in responses {
                let (mut stream, _) = listener.accept().await.expect("request");
                let mut request = Vec::new();
                let mut buffer = vec![0; 65536];
                // Read the headers, then as much body as Content-Length says.
                loop {
                    let size = stream.read(&mut buffer).await.expect("read");
                    request.extend_from_slice(&buffer[..size]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|n| n.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if rest.len() >= length {
                            break;
                        }
                    }
                    if size == 0 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request).to_string();
                assert!(
                    text.lines().next().expect("request line").contains(path),
                    "unexpected request path"
                );
                seen.push(text);
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\nContent-Type: application/json\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.expect("write");
            }
            seen
        });
        let (mut http, unused) = status_response_fixture(vec![]).await;
        unused.abort();
        http.test_base = Some(Url::parse(&format!("http://{address}/")).expect("URL"));
        (http, task)
    }

    #[tokio::test]
    async fn runpod_bill_closes_its_range_and_falls_back_to_the_month_bucket() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![
            (
                "GET /v2/billing?startTime=",
                400,
                r#"{"error":"bad range"}"#,
            ),
            (
                "GET /v2/billing?bucketSize=month&lastN=1",
                200,
                r#"[{"amount":12.5}]"#,
            ),
        ])
        .await;
        let bill = super::super::runpod::Runpod::new(http)
            .provider_bill(&Secret::new("fixture".into()), "acct")
            .await
            .unwrap()
            .unwrap();
        let sent = task.await.unwrap();
        assert!(sent[0].lines().next().unwrap().contains("endTime="));
        assert_eq!(bill.billed.cents, 1250);
    }

    #[tokio::test]
    async fn vercel_bill_sums_the_jsonl_charges() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![(
            "GET /v1/billing/charges?",
            200,
            "{\"BilledCost\":1.25,\"BillingCurrency\":\"USD\",\"ChargePeriodStart\":\"2026-11-02T00:00:00Z\"}\n{\"BilledCost\":\"3.5\",\"BillingCurrency\":\"USD\",\"ChargePeriodStart\":\"2026-11-01T00:00:00Z\"}\n",
        )])
        .await;
        let bill = super::super::vercel::Vercel::new(http)
            .provider_bill(&Secret::new("fixture".into()), "team_1")
            .await
            .unwrap()
            .unwrap();
        let sent = task.await.unwrap();
        assert!(sent[0].contains("teamId=team_1"));
        assert_eq!(bill.billed.cents, 475);
        assert_eq!(bill.period_start.as_deref(), Some("2026-11-01T00:00:00Z"));
    }

    #[tokio::test]
    async fn resend_plan_usage_reads_emails_and_domains() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, _task) = capturing_fixture(vec![(
            "GET /usage",
            200,
            r#"{"emails":{"monthly":{"used":1200,"limit":3000},"daily":{"used":40,"limit":100}},"domains":{"used":1,"limit":1}}"#,
        )])
        .await;
        let usage = super::super::resend::Resend::new(http)
            .provider_plan_usage(&Secret::new("fixture".into()), "acct")
            .await
            .unwrap();
        assert_eq!(usage.len(), 3);
        assert_eq!(usage[0].label, "Emails this month");
        assert_eq!(usage[0].limit, Some(3000.0));
    }

    #[tokio::test]
    async fn render_details_show_variables_domains_and_settings() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![
            (
                "GET /services/srv-1/deploys?limit=25",
                200,
                r#"[{"deploy":{"id":"dep-1","status":"live","createdAt":"2026-11-01T10:00:00Z","commit":{"id":"abc","message":"Ship"}}}]"#,
            ),
            (
                "GET /services/srv-1 ",
                200,
                r#"{"repo":"https://github.com/sam/shop","rootDir":"web","serviceDetails":{"url":"https://shop.onrender.com","region":"frankfurt","numInstances":2,"plan":"starter","healthCheckPath":"/health","envSpecificDetails":{"buildCommand":"npm run build","startCommand":"npm start"},"disk":{"id":"dsk-1","name":"data","mountPath":"/data","sizeGB":1}}}"#,
            ),
            (
                "GET /services/srv-1/env-vars?limit=100",
                200,
                r#"[{"envVar":{"key":"STRIPE_KEY"}},{"envVar":{"key":"DATABASE_URL"}}]"#,
            ),
            (
                "GET /services/srv-1/custom-domains?limit=100",
                200,
                r#"[{"customDomain":{"name":"shop.sam.dev","verificationStatus":"unverified"}}]"#,
            ),
        ])
        .await;
        let details = super::super::render::Render::new(http)
            .provider_details(&Secret::new("fixture".into()), "owner", "srv-1")
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(details.deployments.len(), 1);
        assert_eq!(
            details.variable_names.as_deref(),
            Some(&["DATABASE_URL".to_owned(), "STRIPE_KEY".to_owned()][..])
        );
        assert_eq!(details.domains.len(), 2);
        assert!(!details.domains[0].custom && details.domains[1].custom);
        let config = details.config.unwrap();
        assert_eq!(config.repo.as_deref(), Some("sam/shop"));
        assert_eq!(config.replicas, Some(2));
        assert_eq!(details.volumes[0].mount_path.as_deref(), Some("/data"));
        assert_eq!(details.sleeps, Some(false));
    }

    #[tokio::test]
    async fn cloudflare_worker_details_show_bindings_and_domains() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![
            (
                "/workers/scripts/api/deployments",
                200,
                r#"{"result":{"deployments":[{"id":"d1","created_on":"2026-11-01T10:00:00Z","source":"api"}]}}"#,
            ),
            (
                "/workers/scripts/api/settings",
                200,
                r#"{"result":{"bindings":[{"name":"API_KEY","type":"secret_text"},{"name":"MODE","type":"plain_text"},{"name":"DB","type":"d1"}]}}"#,
            ),
            (
                "/workers/scripts/api/subdomain",
                200,
                r#"{"result":{"enabled":true}}"#,
            ),
            (
                "/workers/subdomain",
                200,
                r#"{"result":{"subdomain":"sam"}}"#,
            ),
            (
                "/workers/domains?service=api",
                200,
                r#"{"result":[{"hostname":"api.sam.dev","service":"api"}]}"#,
            ),
            (
                "/workers/scripts/api/schedules",
                200,
                r#"{"result":{"schedules":[]}}"#,
            ),
        ])
        .await;
        let details = super::super::cloudflare::Cloudflare::new(http)
            .provider_details(&Secret::new("fixture".into()), "acct", "worker:api")
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(
            details.variable_names.as_deref(),
            Some(&["API_KEY".to_owned(), "MODE".to_owned()][..])
        );
        let names: Vec<_> = details
            .domains
            .iter()
            .map(|domain| domain.name.as_str())
            .collect();
        assert_eq!(names, ["api.sam.workers.dev", "api.sam.dev"]);
    }

    #[tokio::test]
    async fn resend_give_key_asks_for_a_sending_key_for_that_domain() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![(
            "POST /api-keys",
            201,
            r#"{"id":"key-1","token":"re_fixture_made"}"#,
        )])
        .await;
        let key = super::super::resend::Resend::new(http)
            .issue_key(&Secret::new("fixture".into()), "team", "dom-1", "sam-site")
            .await
            .unwrap();
        assert_eq!(key.expose(), "re_fixture_made");
        let sent = task.await.unwrap();
        let body = sent[0].split_once("\r\n\r\n").unwrap().1;
        let body: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(body["permission"], "sending_access");
        assert_eq!(body["domain_id"], "dom-1");
        assert_eq!(body["name"], "sam-site");
    }

    #[tokio::test]
    async fn resend_logs_are_that_domains_emails_in_range() {
        use crate::application::ports::providers::ProviderAdapter;
        use crate::domain::logs::{LogHistoryQuery, LogLevel};
        let now = chrono::Utc::now();
        let stamp = |minutes: i64| {
            (now - chrono::Duration::minutes(minutes))
                .format("%Y-%m-%d %H:%M:%S%.6f+00")
                .to_string()
        };
        let emails = serde_json::json!({
            "data": [
                {
                    "from": "Sam <hello@example.com>",
                    "to": ["a@example.org"],
                    "subject": "Welcome",
                    "last_event": "delivered",
                    "created_at": stamp(5),
                },
                {
                    "from": "hello@example.com",
                    "to": ["b@example.org"],
                    "subject": "Reset",
                    "last_event": "bounced",
                    "created_at": stamp(10),
                },
                {
                    "from": "other@elsewhere.dev",
                    "to": ["c@example.org"],
                    "subject": "Not ours",
                    "last_event": "delivered",
                    "created_at": stamp(3),
                },
                {
                    "from": "hello@example.com",
                    "to": ["d@example.org"],
                    "subject": "Too old",
                    "last_event": "delivered",
                    "created_at": stamp(60 * 48),
                },
            ],
        })
        .to_string();
        let emails: &'static str = Box::leak(emails.into_boxed_str());
        let (http, task) = capturing_fixture(vec![
            (
                "GET /domains/dom-1",
                200,
                r#"{"id":"dom-1","name":"Example.com"}"#,
            ),
            ("GET /emails?limit=100", 200, emails),
        ])
        .await;
        let query = LogHistoryQuery {
            from: (now - chrono::Duration::hours(24)).to_rfc3339(),
            to: now.to_rfc3339(),
            query: String::new(),
            cursor: None,
        };
        let history = super::super::resend::Resend::new(http)
            .log_history(&Secret::new("fixture".into()), "team", "dom-1", &query)
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(history.items.len(), 2);
        assert!(history.items[0].message.contains("Welcome"));
        assert!(matches!(history.items[1].level, LogLevel::Error));
        assert!(history.items.iter().all(|item| item.timestamp.is_some()));
    }

    #[tokio::test]
    async fn supabase_give_key_prefers_the_publishable_key() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![(
            "GET /projects/proj1/api-keys",
            200,
            r#"[{"name":"anon","type":"legacy","api_key":"anon-fixture"},{"name":"service_role","type":"legacy","api_key":"service-fixture"},{"name":"default","type":"publishable","api_key":"sb_publishable_fixture"},{"name":"secret","type":"secret","api_key":"sb_secret_fixture"}]"#,
        )])
        .await;
        let key = super::super::supabase::Supabase::new(http)
            .issue_key(&Secret::new("fixture".into()), "org", "proj1/fn", "unused")
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(key.expose(), "sb_publishable_fixture");
    }

    #[tokio::test]
    async fn supabase_give_key_never_falls_back_to_a_secret_key() {
        use crate::application::ports::providers::ProviderAdapter;
        let (http, task) = capturing_fixture(vec![(
            "GET /projects/proj1/api-keys",
            200,
            r#"[{"name":"service_role","type":"legacy","api_key":"service-fixture"}]"#,
        )])
        .await;
        let refused = super::super::supabase::Supabase::new(http)
            .issue_key(&Secret::new("fixture".into()), "org", "proj1", "unused")
            .await
            .unwrap_err();
        task.await.unwrap();
        assert_eq!(refused.code(), "supabase.no_public_key");
    }

    #[tokio::test]
    async fn supabase_delete_calls_the_project_and_function_paths_once_under_v1() {
        use crate::application::ports::providers::ProviderAdapter;
        // The base address already ends in /v1/; the path must not repeat it.
        let (http, task) = capturing_fixture(vec![
            ("DELETE /projects/proj1/functions/hello", 200, "{}"),
            ("DELETE /projects/proj1", 200, "{}"),
        ])
        .await;
        let supabase = super::super::supabase::Supabase::new(http);
        let token = Secret::new("fixture".into());
        supabase
            .delete(&token, "org", "proj1/function:hello")
            .await
            .unwrap();
        supabase.delete(&token, "org", "proj1").await.unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn successful_http_status_does_not_hide_body_failures_or_leak_body_text() {
        for (body, code) in [
            ("private-invalid-json", "provider.invalid_response"),
            (
                r#"{"success":false,"errors":[{"message":"private-provider-error"}]}"#,
                "provider.rejected",
            ),
        ] {
            let (http, task) = status_response_fixture(vec![("/resources", 200, body)]).await;
            let error = http
                .json(
                    Provider::Vercel,
                    &Secret::new("token".into()),
                    Method::GET,
                    &["resources"],
                    &[],
                    None,
                )
                .await
                .unwrap_err();
            task.await.unwrap();
            assert_eq!(error.code(), code);
            let evidence = error
                .provider_response()
                .expect("headers survived body failure");
            assert_eq!(evidence.http_status, 200);
            assert!(evidence.request_id.is_none());
            assert!(!error.to_string().contains("private-"));
            assert!(
                !serde_json::to_string(evidence)
                    .unwrap()
                    .contains("private-")
            );
        }
    }

    fn remaining_inventory() -> Vec<(&'static str, u16, &'static str)> {
        vec![
            (
                "/r2/buckets?|cf-r2-jurisdiction: eu",
                200,
                r#"{"result":{"buckets":[]}}"#,
            ),
            (
                "/r2/buckets?|cf-r2-jurisdiction: us",
                200,
                r#"{"result":{"buckets":[]}}"#,
            ),
            (
                "/r2/buckets?|cf-r2-jurisdiction: fedramp",
                200,
                r#"{"result":{"buckets":[]}}"#,
            ),
            (
                "/r2/buckets?|cf-r2-jurisdiction: fedramp-high",
                200,
                r#"{"result":{"buckets":[]}}"#,
            ),
        ]
    }

    #[tokio::test]
    async fn cloudflare_inventory_follows_jurisdiction_cursors_and_account_zone_pages() {
        use crate::application::ports::providers::ProviderAdapter;
        use crate::domain::model::ResourceKind;
        let mut replies = vec![
            ("/workers/scripts?", 200, r#"{"result":[]}"#),
            ("/pages/projects?", 200, r#"{"result":[]}"#),
            ("/d1/database?", 200, r#"{"result":[]}"#),
            ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
            (
                "/r2/buckets?|cf-r2-jurisdiction: default",
                200,
                r#"{"result":{"buckets":[{"name":"uploads","jurisdiction":"default","location":"weur"}]},"result_info":{"cursor":"next/token","total_count":2}}"#,
            ),
            (
                "cursor=next%2Ftoken|cf-r2-jurisdiction: default",
                200,
                r#"{"result":{"buckets":[{"name":"archives"}]},"result_info":{"total_count":2}}"#,
            ),
        ];
        replies.extend(remaining_inventory());
        replies.extend([
            (
                "/zones?account.id=account&page=1&per_page=50",
                200,
                r#"{"result":[{"id":"zone1","name":"example.com","account":{"id":"account"}}],"result_info":{"page":1,"total_pages":2}}"#,
            ),
            (
                "/zones?account.id=account&page=2&per_page=50",
                200,
                r#"{"result":[{"id":"zone2","name":"internal","type":"internal","account":{"id":"account"}}],"result_info":{"page":2,"total_pages":2}}"#,
            ),
        ]);
        let (http, task) = status_response_fixture(replies).await;
        let adapter = super::super::cloudflare::Cloudflare::new(http);
        let token = Secret::new("fixture".into());
        let report = adapter.discover_report(&token, "account").await.unwrap();
        assert!(report.warnings.is_empty());
        assert_eq!(report.resources.len(), 4);
        assert_eq!(report.resources[0].reference, "r2:default:uploads");
        assert_eq!(report.resources[0].kind, ResourceKind::Bucket);
        assert_eq!(report.resources[0].region.as_deref(), Some("weur"));
        assert_eq!(report.resources[2].reference, "zone:zone1");
        assert_eq!(report.resources[2].kind, ResourceKind::Domain);
        assert!(
            report
                .resources
                .iter()
                .all(|resource| resource.url.is_none())
        );
        task.await.unwrap();
        // A bucket outside the default jurisdiction isn't deleted, and a
        // domain takes no Worker secret.
        assert!(matches!(
            adapter
                .delete(&token, "account", "r2:eu:uploads")
                .await,
            Err(EngineError::Provider { code, .. }) if code == "provider.not_supported"
        ));
        assert!(matches!(
            adapter
                .set_secret(&token, "account", "zone:zone1", "KEY", &token)
                .await,
            Err(EngineError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn cloudflare_inventory_rejects_ambiguous_pages_and_preserves_other_families() {
        use crate::application::ports::providers::ProviderAdapter;
        for bucket_response in [
            r#"{"result":{"buckets":[{"name":"uploads","jurisdiction":"eu"}]}}"#,
            r#"{"result":{"buckets":[{"name":"uploads"},{"name":"uploads"}]}}"#,
            r#"{"result":{"buckets":[{"name":"uploads"}]},"result_info":{"total_count":2}}"#,
            r#"{"result":{"buckets":[]},"result_info":{"cursor":"loop"}}"#,
        ] {
            let mut replies = vec![
                (
                    "/workers/scripts?",
                    200,
                    r#"{"result":[{"id":"retained-worker"}]}"#,
                ),
                (
                    "/workers/subdomain",
                    200,
                    r#"{"result":{"subdomain":"acct"}}"#,
                ),
                (
                    "/workers/scripts/retained-worker/subdomain",
                    200,
                    r#"{"result":{"enabled":true}}"#,
                ),
                ("/pages/projects?", 200, r#"{"result":[]}"#),
                ("/d1/database?", 200, r#"{"result":[]}"#),
                ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
                ("/r2/buckets?", 200, bucket_response),
            ];
            replies.extend(remaining_inventory());
            replies.push((
                "/zones?",
                200,
                r#"{"result":[{"id":"zone1","name":"example.com","account":{"id":"foreign"}}]}"#,
            ));
            let (http, task) = status_response_fixture(replies).await;
            let report = super::super::cloudflare::Cloudflare::new(http)
                .discover_report(&Secret::new("fixture".into()), "account")
                .await
                .unwrap();
            assert_eq!(report.resources.len(), 1);
            assert_eq!(report.warnings.len(), 2);
            assert!(
                report
                    .warnings
                    .iter()
                    .all(|warning| warning.code == "provider.invalid_response")
            );
            task.await.unwrap();
        }
        let mut replies = vec![
            ("/workers/scripts?", 200, r#"{"result":[]}"#),
            ("/pages/projects?", 200, r#"{"result":[]}"#),
            ("/d1/database?", 200, r#"{"result":[]}"#),
            ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
            ("/r2/buckets?", 403, "private-provider-response"),
        ];
        replies.extend(remaining_inventory());
        replies.push(("/zones?", 403, "private-provider-response"));
        let (http, task) = status_response_fixture(replies).await;
        let report = super::super::cloudflare::Cloudflare::new(http)
            .discover_report(&Secret::new("fixture".into()), "account")
            .await
            .unwrap();
        assert_eq!(report.warnings.len(), 2);
        assert!(
            report
                .warnings
                .iter()
                .all(|warning| warning.code == "provider.token_scope"
                    && !warning.message.contains("private-provider-response"))
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cloudflare_inventory_bounds_repeated_cursors_and_stops_on_expired_tokens() {
        use crate::application::ports::providers::ProviderAdapter;
        let mut replies = vec![
            ("/workers/scripts?", 200, r#"{"result":[]}"#),
            ("/pages/projects?", 200, r#"{"result":[]}"#),
            ("/d1/database?", 200, r#"{"result":[]}"#),
            ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
            (
                "/r2/buckets?",
                200,
                r#"{"result":{"buckets":[{"name":"uploads"}]},"result_info":{"cursor":"same"}}"#,
            ),
            (
                "cursor=same",
                200,
                r#"{"result":{"buckets":[{"name":"archives"}]},"result_info":{"cursor":"same"}}"#,
            ),
        ];
        replies.extend(remaining_inventory());
        replies.push(("/zones?", 200, r#"{"result":[]}"#));
        let (http, task) = status_response_fixture(replies).await;
        let report = super::super::cloudflare::Cloudflare::new(http)
            .discover_report(&Secret::new("fixture".into()), "account")
            .await
            .unwrap();
        assert!(report.resources.is_empty());
        assert_eq!(report.warnings.len(), 1);
        assert_eq!(report.warnings[0].resource_type, "r2:default");
        task.await.unwrap();
        let (http, task) = status_response_fixture(vec![
            ("/workers/scripts?", 200, r#"{"result":[]}"#),
            ("/pages/projects?", 200, r#"{"result":[]}"#),
            ("/d1/database?", 200, r#"{"result":[]}"#),
            ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
            ("/r2/buckets?", 401, "private-provider-response"),
        ])
        .await;
        let error = super::super::cloudflare::Cloudflare::new(http)
            .discover_report(&Secret::new("fixture".into()), "account")
            .await
            .err()
            .unwrap();
        assert_eq!(error.code(), "provider.token_expired");
        task.await.unwrap();
    }

    #[tokio::test]
    async fn cloudflare_discovery_reports_each_inaccessible_family_without_hiding_auth_failure() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        for (workers, pages) in [(200, 403), (403, 200), (403, 403), (200, 200)] {
            let mut replies = vec![(
                "/workers/scripts?",
                workers,
                r#"{"result":[{"id":"worker-one"}]}"#,
            )];
            // Listed Workers bring a read of the account's workers.dev subdomain.
            if workers == 200 {
                replies.push((
                    "/workers/subdomain",
                    200,
                    r#"{"result":{"subdomain":"acct"}}"#,
                ));
                replies.push((
                    "/workers/scripts/worker-one/subdomain",
                    200,
                    r#"{"result":{"enabled":true}}"#,
                ));
            }
            let (http, task) = status_response_fixture(
                [
                    replies,
                    vec![
                        (
                            "/pages/projects?",
                            pages,
                            r#"{"result":[{"name":"pages-one","subdomain":"pages-one.pages.dev"}]}"#,
                        ),
                        ("/d1/database?", 200, r#"{"result":[]}"#),
                        ("/storage/kv/namespaces?", 200, r#"{"result":[]}"#),
                        (
                            "/r2/buckets?",
                            200,
                            r#"{"result":{"buckets":[]},"result_info":{"total_count":0}}"#,
                        ),
                        (
                            "/r2/buckets?",
                            200,
                            r#"{"result":{"buckets":[]},"result_info":{"total_count":0}}"#,
                        ),
                        (
                            "/r2/buckets?",
                            200,
                            r#"{"result":{"buckets":[]},"result_info":{"total_count":0}}"#,
                        ),
                        (
                            "/r2/buckets?",
                            200,
                            r#"{"result":{"buckets":[]},"result_info":{"total_count":0}}"#,
                        ),
                        (
                            "/r2/buckets?",
                            200,
                            r#"{"result":{"buckets":[]},"result_info":{"total_count":0}}"#,
                        ),
                        (
                            "/zones?account.id=account",
                            200,
                            r#"{"result":[],"result_info":{"page":1,"total_pages":0}}"#,
                        ),
                    ],
                ]
                .concat(),
            )
            .await;
            let report = super::super::cloudflare::Cloudflare::new(http)
                .discover_report(&token, "account")
                .await
                .expect("report");
            let inaccessible = usize::from(workers == 403) + usize::from(pages == 403);
            assert_eq!(report.resources.len(), 2 - inaccessible);
            assert_eq!(report.warnings.len(), inaccessible);
            assert!(
                report
                    .warnings
                    .iter()
                    .all(|warning| warning.code == "provider.token_scope")
            );
            if workers == 403 {
                assert!(
                    report
                        .warnings
                        .iter()
                        .any(|warning| warning.resource_type == "workers")
                );
            }
            if pages == 403 {
                assert!(
                    report
                        .warnings
                        .iter()
                        .any(|warning| warning.resource_type == "pages")
                );
            }
            task.await.expect("fixture");
        }
        let (http, task) =
            status_response_fixture(vec![("/workers/scripts?", 401, "private-body")]).await;
        let error = super::super::cloudflare::Cloudflare::new(http)
            .discover_report(&token, "account")
            .await
            .err()
            .expect("authentication failure");
        assert_eq!(error.code(), "provider.token_expired");
        assert!(!error.to_string().contains("private-body"));
        task.await.expect("fixture");
        for (status, body, expected) in [
            (401, "private-body", "provider.token_expired"),
            (200, "{}", "provider.invalid_response"),
        ] {
            let (http, task) = status_response_fixture(vec![
                (
                    "/workers/scripts?",
                    200,
                    r#"{"result":[{"id":"observed"}]}"#,
                ),
                (
                    "/workers/subdomain",
                    200,
                    r#"{"result":{"subdomain":"acct"}}"#,
                ),
                (
                    "/workers/scripts/observed/subdomain",
                    200,
                    r#"{"result":{"enabled":true}}"#,
                ),
                ("/pages/projects?", status, body),
            ])
            .await;
            let error = super::super::cloudflare::Cloudflare::new(http)
                .discover_report(&token, "account")
                .await
                .err()
                .expect("failed scan");
            assert_eq!(error.code(), expected);
            task.await.expect("fixture");
        }
    }

    #[tokio::test]
    async fn neon_follows_cursors_and_rejects_partial_and_repeated_pages() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        let first = r#"{"projects":[{"id":"one","name":"one"}],"pagination":{"cursor":"one"}}"#;
        let last = r#"{"projects":[{"id":"two","name":"two"}],"pagination":{}}"#;
        let (http, task) =
            response_fixture(vec![("/projects?", first), ("cursor=one", last)]).await;
        let result = super::super::neon::Neon::new(http)
            .discover(&token, "org")
            .await
            .expect("pages");
        assert_eq!(result.len(), 2);
        task.await.expect("fixture");
        let (http, task) = response_fixture(vec![(
            "/projects?",
            r#"{"projects":[],"unavailable":["hidden"]}"#,
        )])
        .await;
        assert!(
            super::super::neon::Neon::new(http)
                .discover(&token, "org")
                .await
                .is_err()
        );
        task.await.expect("fixture");
        let (http, task) =
            response_fixture(vec![("/projects?", first), ("cursor=one", first)]).await;
        assert!(
            super::super::neon::Neon::new(http)
                .discover(&token, "org")
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn neon_export_covers_all_branch_databases_and_requests_direct_connections() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture".into());
        let (http, task) = response_fixture(vec![
            ("/projects/project", r#"{"project":{"org_id":"org"}}"#),
            (
                "/branches?limit=100",
                r#"{"branches":[{"id":"br-a","project_id":"project"}],"pagination":{"next":"second"}}"#,
            ),
            (
                "/br-a/databases",
                r#"{"databases":[{"branch_id":"br-a","name":"main","owner_name":"owner"},{"branch_id":"br-a","name":"extra","owner_name":"other"}]}"#,
            ),
            (
                "cursor=second",
                r#"{"branches":[{"id":"br-b","project_id":"project"}],"pagination":{"next":null}}"#,
            ),
            (
                "/br-b/databases",
                r#"{"databases":[{"branch_id":"br-b","name":"main","owner_name":"owner"}]}"#,
            ),
            (
                "branch_id=br-a&database_name=extra&role_name=other&pooled=false",
                r#"{"uri":"postgresql://other:private@host/extra"}"#,
            ),
        ])
        .await;
        let neon = super::super::neon::Neon::new(http);
        let inventory = neon
            .database_inventory(&token, "org", "project")
            .await
            .expect("complete inventory");
        assert_eq!(inventory.len(), 3);
        assert_eq!(inventory[0].database_name, "extra");
        assert_eq!(inventory[2].branch_id, "br-b");
        let connection = neon
            .database_export_connection(&token, "org", "project", &inventory[0])
            .await
            .expect("direct connection");
        assert_eq!(connection.expose(), "postgresql://other:private@host/extra");
        task.await.expect("fixture");
        let (http, task) = response_fixture(vec![
            ("/projects/project", r#"{"project":{"org_id":"org"}}"#),
            (
                "/branches?",
                r#"{"branches":[{"id":"br-empty","project_id":"project"}]}"#,
            ),
            ("/databases", r#"{"databases":[]}"#),
        ])
        .await;
        assert!(
            super::super::neon::Neon::new(http)
                .database_inventory(&token, "org", "project")
                .await
                .expect("known empty database inventory")
                .is_empty()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn neon_export_rejects_inaccessible_duplicate_or_foreign_inventory() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture".into());
        for database_response in [
            r#"{"databases":[{"branch_id":"foreign","name":"main","owner_name":"owner"}]}"#,
            r#"{"databases":[{"branch_id":"br-a","name":"main","owner_name":"owner"},{"branch_id":"br-a","name":"main","owner_name":"owner"}]}"#,
            r#"{"databases":[],"pagination":{"next":"more"}}"#,
        ] {
            let (http, task) = response_fixture(vec![
                ("/projects/project", r#"{"project":{"org_id":"org"}}"#),
                (
                    "/branches?",
                    r#"{"branches":[{"id":"br-a","project_id":"project"}]}"#,
                ),
                ("/databases", database_response),
            ])
            .await;
            assert!(
                super::super::neon::Neon::new(http)
                    .database_inventory(&token, "org", "project")
                    .await
                    .is_err()
            );
            task.await.expect("fixture");
        }
        let (http, task) = status_response_fixture(vec![
            ("/projects/project", 200, r#"{"project":{"org_id":"org"}}"#),
            (
                "/branches?",
                200,
                r#"{"branches":[{"id":"br-a","project_id":"project"},{"id":"br-b","project_id":"project"}]}"#,
            ),
            (
                "/br-a/databases",
                200,
                r#"{"databases":[{"branch_id":"br-a","name":"main","owner_name":"owner"}]}"#,
            ),
            ("/br-b/databases", 403, r#"{"message":"denied"}"#),
        ])
        .await;
        assert!(
            super::super::neon::Neon::new(http)
                .database_inventory(&token, "org", "project")
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn a_cloudflare_sign_in_token_is_checked_by_reading_the_account() {
        use crate::application::ports::providers::ProviderAdapter as _;
        let token = Secret::new("oauth-access".into());
        // Neither verify address knows a sign-in token; reading Workers works.
        let (http, task) = status_response_fixture(vec![
            (
                "/user/tokens/verify",
                401,
                r#"{"success":false,"errors":[{"code":1000,"message":"Invalid API Token"}]}"#,
            ),
            (
                "/accounts/acct-1/tokens/verify",
                401,
                r#"{"success":false,"errors":[{"code":1000,"message":"Invalid API Token"}]}"#,
            ),
            (
                "/accounts/acct-1/workers/scripts",
                200,
                r#"{"success":true,"result":[],"result_info":{"total_pages":1}}"#,
            ),
        ])
        .await;
        let scopes = super::super::cloudflare::Cloudflare::new(http.clone())
            .verify(&token, "acct-1")
            .await
            .expect("signed in");
        assert!(
            scopes
                .iter()
                .any(|scope| scope.starts_with("Signed in with Cloudflare"))
        );
        task.await.expect("fixture");
        // A token that can't read the account still fails.
        let (http, task) = status_response_fixture(vec![
            (
                "/user/tokens/verify",
                401,
                r#"{"success":false,"errors":[{"code":1000,"message":"Invalid API Token"}]}"#,
            ),
            (
                "/accounts/acct-1/tokens/verify",
                401,
                r#"{"success":false,"errors":[{"code":1000,"message":"Invalid API Token"}]}"#,
            ),
            (
                "/accounts/acct-1/workers/scripts",
                401,
                r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}]}"#,
            ),
        ])
        .await;
        assert!(
            super::super::cloudflare::Cloudflare::new(http)
                .verify(&token, "acct-1")
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn a_cloudflare_scan_skips_what_it_cant_list_and_keeps_going() {
        use crate::application::ports::providers::ProviderAdapter as _;
        let token = Secret::new("oauth-access".into());
        let denied = r#"{"success":false,"errors":[{"code":10000,"message":"denied"}]}"#;
        let (http, task) = status_response_fixture(vec![
            (
                "/accounts/acct-1/workers/scripts?page=1&per_page=100",
                200,
                r#"{"success":true,"result":[{"id":"site"}],"result_info":{"total_pages":1}}"#,
            ),
            // The account's workers.dev subdomain, for the Worker's address.
            (
                "/workers/subdomain",
                200,
                r#"{"result":{"subdomain":"acct"}}"#,
            ),
            (
                "/workers/scripts/site/subdomain",
                200,
                r#"{"result":{"enabled":true}}"#,
            ),
            // Pages lists take at most 10 a page; a refusal is noted, not fatal.
            (
                "/accounts/acct-1/pages/projects?page=1&per_page=10",
                400,
                r#"{"success":false,"errors":[{"code":8000000,"message":"bad"}]}"#,
            ),
            (
                "/accounts/acct-1/d1/database?page=1&per_page=100",
                403,
                denied,
            ),
            (
                "/accounts/acct-1/storage/kv/namespaces?page=1&per_page=100",
                403,
                denied,
            ),
            ("/r2/buckets", 403, denied),
            ("/r2/buckets", 403, denied),
            ("/r2/buckets", 403, denied),
            ("/r2/buckets", 403, denied),
            ("/r2/buckets", 403, denied),
            ("/zones", 403, denied),
        ])
        .await;
        let report = super::super::cloudflare::Cloudflare::new(http)
            .discover_report(&token, "acct-1")
            .await
            .expect("scan");
        assert_eq!(report.resources.len(), 1);
        assert_eq!(report.resources[0].reference, "worker:site");
        assert_eq!(
            report.resources[0].url.as_deref(),
            Some("https://site.acct.workers.dev")
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.resource_type == "pages")
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn cloudflare_reports_the_state_of_watched_domains_and_buckets() {
        use crate::application::ports::providers::ProviderAdapter as _;
        use crate::domain::model::{Mode, Resource, ResourceKind, ResourceStatus};
        let token = Secret::new("oauth-access".into());
        let watched = |reference: &str, kind| Resource {
            id: "r1".into(),
            project_id: None,
            kind,
            provider: crate::Provider::Cloudflare,
            account_id: "a1".into(),
            provider_ref: reference.into(),
            name: "x".into(),
            region: None,
            status: ResourceStatus::Unknown,
            mode: Mode::Watching,
            url: None,
            group: None,
        };
        let (http, task) = status_response_fixture(vec![
            (
                "/zones/z1 ",
                200,
                r#"{"success":true,"result":{"id":"z1","status":"active"}}"#,
            ),
            (
                "/accounts/acct-1/r2/buckets/assets ",
                404,
                r#"{"success":false,"errors":[{"code":10006,"message":"The specified bucket does not exist."}]}"#,
            ),
            (
                "/zones/z2 ",
                200,
                r#"{"success":true,"result":{"id":"z2","status":"pending"}}"#,
            ),
        ])
        .await;
        let cloudflare = super::super::cloudflare::Cloudflare::new(http);
        let zone = cloudflare
            .observe_resource(&token, "acct-1", &watched("zone:z1", ResourceKind::Domain))
            .await
            .expect("zone");
        assert_eq!(zone.status, ResourceStatus::Live);
        let bucket = cloudflare
            .observe_resource(
                &token,
                "acct-1",
                &watched("r2:default:assets", ResourceKind::Bucket),
            )
            .await
            .expect("bucket");
        assert_eq!(bucket.status, ResourceStatus::Stopped);
        let pending = cloudflare
            .observe_resource(&token, "acct-1", &watched("zone:z2", ResourceKind::Domain))
            .await
            .expect("pending zone");
        assert_eq!(pending.status, ResourceStatus::Deploying);
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn cloudflare_api_operations_are_aimed_at_the_resource() {
        use crate::application::ports::providers::ProviderAdapter as _;
        let token = Secret::new("oauth-access".into());
        let (http, task) = response_fixture(vec![
            (
                "GET /accounts/acct-1/r2/buckets/assets ",
                r#"{"success":true,"result":{"name":"assets","location":"WEUR"}}"#,
            ),
            (
                "GET /zones/z1/dns_records",
                r#"{"success":true,"result":[{"id":"r1","type":"CNAME","name":"api.sam.dev"}]}"#,
            ),
        ])
        .await;
        let cloudflare = super::super::cloudflare::Cloudflare::new(http);
        let bucket = cloudflare
            .provider_operation(
                &token,
                "acct-1",
                Some("r2:default:assets"),
                None,
                "r2GetBucket",
                serde_json::json!({}),
            )
            .await
            .expect("bucket");
        assert!(bucket.to_string().contains("WEUR"));
        cloudflare
            .provider_operation(
                &token,
                "acct-1",
                Some("zone:z1"),
                None,
                "dnsRecordsForAZoneListDnsRecords",
                serde_json::json!({}),
            )
            .await
            .expect("dns");
        // A bucket kept in another jurisdiction isn't reached by these.
        assert!(
            cloudflare
                .provider_operation(
                    &token,
                    "acct-1",
                    Some("r2:eu:assets"),
                    None,
                    "r2GetBucket",
                    serde_json::json!({})
                )
                .await
                .is_err()
        );
        // Another account can't be named instead of the connected one.
        assert!(
            cloudflare
                .provider_operation(
                    &token,
                    "acct-1",
                    None,
                    None,
                    "d1ListDatabases",
                    serde_json::json!({"account_id": "someone-else"})
                )
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn neon_api_operations_are_aimed_at_the_project() {
        use crate::application::ports::providers::ProviderAdapter as _;
        let token = Secret::new("napi_token".into());
        let (http, task) = response_fixture(vec![(
            "GET /projects/proj-1/branches ",
            r#"{"branches":[{"id":"br-main","name":"main","default":true}]}"#,
        )])
        .await;
        let neon = super::super::neon::Neon::new(http);
        let branches = neon
            .provider_operation(
                &token,
                "org-1",
                Some("proj-1"),
                None,
                "listProjectBranches",
                serde_json::json!({}),
            )
            .await
            .expect("branches");
        assert!(branches.to_string().contains("br-main"));
        assert!(
            neon.provider_operation(
                &token,
                "org-1",
                Some("proj-1"),
                None,
                "listProjectBranches",
                serde_json::json!({"project_id": "proj-2"})
            )
            .await
            .is_err()
        );
        assert!(
            neon.provider_operations()
                .iter()
                .any(|op| op.name == "neon branches list")
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn manifest_api_operations_are_aimed_at_the_resource() {
        use crate::application::ports::providers::ProviderAdapter as _;
        let token = Secret::new("vc_token".into());
        let (http, task) = response_fixture(vec![
            (
                "GET /v9/projects/prj_1?teamId=team_1 ",
                r#"{"id":"prj_1","name":"site","env":{"KEY":"value"},"apiKey":"k"}"#,
            ),
            // The fixture stands in for the provider's whole base address, `/v1` included.
            (
                "GET /apps/notes-api/machines ",
                r#"[{"id":"m1","state":"started"}]"#,
            ),
        ])
        .await;
        let vercel = super::super::vercel::Vercel::new(http.clone());
        let project = vercel
            .provider_operation(
                &token,
                "team_1",
                Some("prj_1"),
                None,
                "getProject",
                Value::Null,
            )
            .await
            .expect("project");
        assert_eq!(project["name"], "site");
        // Secrets and variable values never come back.
        assert!(project.get("apiKey").is_none());
        assert_eq!(project["env"]["KEY"], Value::Null);
        let fly = super::super::fly::Fly::new(http);
        let machines = fly
            .provider_operation(
                &token,
                "sam-org",
                Some("notes-api"),
                None,
                "machinesList",
                Value::Null,
            )
            .await
            .expect("machines");
        assert_eq!(machines[0]["state"], "started");
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn vercel_pages_supabase_organization_and_fly_truncation_are_verified() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            (
                "/v10/projects?",
                r#"{"projects":[{"id":"a","name":"a"}],"pagination":{"next":123}}"#,
            ),
            (
                "from=123",
                r#"{"projects":[{"id":"b","name":"b"}],"pagination":{"next":null}}"#,
            ),
        ])
        .await;
        assert_eq!(
            super::super::vercel::Vercel::new(http)
                .discover(&token, "team")
                .await
                .expect("projects")
                .len(),
            2
        );
        task.await.expect("fixture");
        let (http, task) = response_fixture(vec![(
            "/projects",
            r#"[{"id":"a","name":"a","organization_id":"ours","status":"ACTIVE_HEALTHY"},{"id":"b","name":"b","organization_id":"other"}]"#,
        )])
        .await;
        assert_eq!(
            super::super::supabase::Supabase::new(http)
                .discover(&token, "ours")
                .await
                .expect("projects")
                .len(),
            1
        );
        task.await.expect("fixture");
        let (http, task) = response_fixture(vec![(
            "/apps?org_slug=ours",
            r#"{"apps":[],"total_apps":2}"#,
        )])
        .await;
        assert!(
            super::super::fly::Fly::new(http)
                .discover(&token, "ours")
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_services_report_state_logs_and_measured_usage() {
        use crate::{
            application::ports::providers::ProviderAdapter,
            domain::{logs::LogLevel, model::*},
        };
        const LATEST: &str = r#"{"data":{"deployments":{"edges":[{"node":{"id":"d1","status":"SUCCESS","url":"api.up.railway.app","staticUrl":null,"environmentId":"e1"}}]}}}"#;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            (
                "/",
                r#"{"data":{"projects":{"edges":[{"node":{"id":"p1","name":"P"}}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}"#,
            ),
            (
                "/",
                r#"{"data":{"project":{"services":{"edges":[{"node":{"id":"s1","name":"api"}}],"pageInfo":{"hasNextPage":false}}}}}"#,
            ),
            ("/", LATEST),
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"deploymentLogs":[{"timestamp":"2026-11-03T10:00:00Z","message":"boot with fixture-token","severity":"error"}]}}"#,
            ),
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"metrics":[{"measurement":"CPU_USAGE","values":[{"ts":0,"value":100.0},{"ts":300,"value":100.0}]},{"measurement":"MEMORY_USAGE_GB","values":[{"ts":1,"value":0.0}]},{"measurement":"DISK_USAGE_GB","values":[{"ts":1,"value":2.5}]}]}}"#,
            ),
            (
                "/",
                r#"{"data":{"deployments":{"edges":[{"node":{"id":"d1","createdAt":"2026-10-01T00:00:00Z"}}]}}}"#,
            ),
            (
                "/",
                r#"{"data":{"httpLogs":[{"timestamp":"2026-11-01T10:00:00Z","requestId":"a","httpStatus":200},{"timestamp":"2026-11-01T10:00:01Z","requestId":"b","httpStatus":502},{"timestamp":"2026-11-01T10:00:01Z","requestId":"b","httpStatus":502},{"timestamp":"2026-11-01T11:00:00Z","requestId":"c","httpStatus":404}]}}"#,
            ),
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"deploymentLogs":[{"timestamp":"2026-11-03T10:00:01Z","message":"listening","severity":"info"}]}}"#,
            ),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        let capabilities = railway.capabilities();
        assert!(capabilities.logs && capabilities.usage && !capabilities.deploy);
        let found = railway.discover(&token, "").await.expect("services");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].reference, "p1/s1");
        assert_eq!(found[0].status, ResourceStatus::Live);
        assert_eq!(found[0].url.as_deref(), Some("https://api.up.railway.app"));
        let history = railway
            .log_history(
                &token,
                "",
                "p1/s1",
                &crate::domain::logs::LogHistoryQuery {
                    from: "2026-11-03T00:00:00Z".into(),
                    to: "2026-11-03T12:00:00Z".into(),
                    query: String::new(),
                    cursor: None,
                },
            )
            .await
            .expect("history");
        assert_eq!(history.items.len(), 1);
        assert_eq!(history.items[0].level, LogLevel::Error);
        assert!(!history.items[0].message.contains("fixture-token"));
        assert!(!history.coverage_complete);
        let resource = Resource {
            id: "resource".into(),
            project_id: Some("project".into()),
            kind: ResourceKind::Service,
            provider: Provider::Railway,
            account_id: "account".into(),
            provider_ref: "p1/s1".into(),
            name: "api".into(),
            region: None,
            status: ResourceStatus::Live,
            mode: Mode::Watching,
            url: None,
            group: None,
        };
        let usage = railway
            .usage(&token, "", &resource, "2026-11-01")
            .await
            .expect("usage");
        assert_eq!(usage.compute_seconds, Some(60_000));
        assert_eq!(usage.storage_bytes, Some(2_500_000_000));
        // Counted from edge request logs: a repeated line counts once, only 5xx are errors.
        assert_eq!(usage.requests, Some(3));
        assert_eq!(usage.errors, Some(1));
        let mut tail = railway
            .open_logs(&token, "", "p1/s1", "")
            .await
            .expect("tail");
        let line = tail.next().await.expect("line").expect("entry");
        assert_eq!(line.message, "listening");
        tail.close().await.expect("close");
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_runs_any_catalog_operation_with_context_filled_in() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            // The service's environment comes from its newest deployment.
            (
                "/",
                r#"{"data":{"deployments":{"edges":[{"node":{"id":"d1","status":"SUCCESS","url":null,"staticUrl":null,"environmentId":"e1"}}]}}}"#,
            ),
            (
                "/",
                r#"{"data":{"httpDurationMetrics":{"samples":[{"ts":1,"p50":40,"p90":90,"p95":180,"p99":640}]}}}"#,
            ),
            (
                "/",
                r#"{"data":{"deploymentCancel":null},"errors":[{"message":"Deployment is not building"}]}"#,
            ),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        let answer = railway
            .provider_operation(
                &token,
                "ws1",
                Some("p1/s1"), None,
                "httpDurationMetrics",
                serde_json::json!({"startDate":"2026-11-02T00:00:00Z","endDate":"2026-11-03T00:00:00Z"}),
            )
            .await
            .expect("metrics");
        assert_eq!(answer["samples"][0]["p95"], 180);
        let refused = railway
            .provider_operation(
                &token,
                "ws1",
                Some("p1/s1"),
                None,
                "deploymentCancel",
                serde_json::json!({"id":"d1"}),
            )
            .await
            .expect_err("refused");
        assert!(
            refused.to_string().contains("not building")
                || format!("{refused:?}").contains("not building")
        );
        // Unknown arguments and missing ones are refused before anything is sent.
        assert!(
            railway
                .provider_operation(
                    &token,
                    "ws1",
                    Some("p1/s1"),
                    None,
                    "deploymentCancel",
                    serde_json::json!({"nope":1})
                )
                .await
                .is_err()
        );
        assert!(
            railway
                .provider_operation(
                    &token,
                    "ws1",
                    None,
                    None,
                    "volumeInstanceBackupList",
                    serde_json::Value::Null
                )
                .await
                .is_err()
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_reports_the_workspace_bill_not_an_estimate() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![(
            "/",
            r#"{"data":{"workspace":{"customer":{"currentUsage":7.4321,"billingPeriod":{"start":"2026-11-01T00:00:00Z","end":"2026-12-01T00:00:00Z"},"usageLimit":{"softLimit":20,"hardLimit":null,"isOverLimit":false}}}}}"#,
        )])
        .await;
        let bill = super::super::Railway::new(http)
            .provider_bill(&token, "ws1")
            .await
            .expect("bill")
            .expect("a bill");
        assert_eq!(bill.billed.cents, 743);
        assert_eq!(bill.period_end.as_deref(), Some("2026-12-01T00:00:00Z"));
        assert_eq!(bill.soft_limit.map(|m| m.cents), Some(2000));
        assert_eq!(bill.hard_limit, None);
        assert!(!bill.over_limit);
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_provider_details_keep_names_and_drop_variable_values() {
        use crate::application::ports::providers::{ProviderAdapter, ProviderDeployState};
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            (
                "/",
                r#"{"data":{"deployments":{"edges":[{"node":{"id":"d2","status":"SUCCESS","createdAt":"2026-11-02T10:00:00Z","url":null,"staticUrl":"api.up.railway.app","environmentId":"e1","meta":{"commitHash":"abc123","commitMessage":"Fix login","branch":"main"}}},{"node":{"id":"d1","status":"CRASHED","createdAt":"2026-11-01T10:00:00Z","url":null,"staticUrl":null,"environmentId":"e1","meta":null}}]}}}"#,
            ),
            (
                "/",
                r#"{"data":{"variables":{"DATABASE_URL":"${{Postgres.DATABASE_URL}}","API_KEY":"sk-secret-value"}}}"#,
            ),
            (
                "/",
                r#"{"data":{"variables":{"SHARED_TOKEN":"shared-secret-value"}}}"#,
            ),
            (
                "/",
                r#"{"data":{"serviceInstance":{"sleepApplication":true}}}"#,
            ),
            (
                "/",
                r#"{"data":{"domains":{"serviceDomains":[{"domain":"api.up.railway.app"}],"customDomains":[{"domain":"api.sam.dev","status":{"certificateStatus":"CERTIFICATE_STATUS_TYPE_VALID","dnsRecords":[{"fqdn":"api.sam.dev","recordType":"DNS_RECORD_TYPE_CNAME","requiredValue":"x.up.railway.app","currentValue":"x.up.railway.app","status":"DNS_RECORD_STATUS_PROPAGATED"}]}},{"domain":"new.sam.dev","status":{"certificateStatus":"CERTIFICATE_STATUS_TYPE_VALIDATING_OWNERSHIP","dnsRecords":[{"fqdn":"new.sam.dev","recordType":"DNS_RECORD_TYPE_CNAME","requiredValue":"y.up.railway.app","currentValue":"","status":"DNS_RECORD_STATUS_REQUIRES_UPDATE"}]}}]}}}"#,
            ),
        ])
        .await;
        let details = super::super::Railway::new(http)
            .provider_details(&token, "", "p1/s1")
            .await
            .expect("details");
        assert_eq!(details.deployments.len(), 2);
        assert_eq!(details.deployments[0].state, ProviderDeployState::Succeeded);
        assert_eq!(details.deployments[0].commit.as_deref(), Some("abc123"));
        assert_eq!(details.deployments[1].state, ProviderDeployState::Failed);
        assert_eq!(
            details.variable_names,
            Some(vec!["API_KEY".to_owned(), "DATABASE_URL".to_owned()])
        );
        assert!(
            !serde_json::to_string(&details)
                .unwrap()
                .contains("secret-value")
        );
        assert_eq!(
            details.shared_variable_names,
            Some(vec!["SHARED_TOKEN".to_owned()])
        );
        assert!(
            !serde_json::to_string(&details)
                .unwrap()
                .contains("shared-secret-value")
        );
        assert_eq!(details.sleeps, Some(true));
        assert_eq!(details.domains[0].name, "api.sam.dev");
        assert!(details.domains[0].custom);
        assert_eq!(
            details.domains[0].state,
            Some(crate::application::ports::providers::ProviderDomainState::Working)
        );
        assert_eq!(
            details.domains[1].state,
            Some(crate::application::ports::providers::ProviderDomainState::Pending)
        );
        assert_eq!(details.domains[1].records[0].kind, "CNAME");
        assert_eq!(details.domains[1].records[0].value, "y.up.railway.app");
        assert_eq!(
            details.domains[2].state,
            Some(crate::application::ports::providers::ProviderDomainState::Working)
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_variables_are_set_and_deleted_on_the_service() {
        use crate::application::ports::providers::ProviderAdapter;
        const LATEST: &str = r#"{"data":{"deployments":{"edges":[{"node":{"id":"d1","status":"SUCCESS","url":null,"staticUrl":null,"environmentId":"e1"}}]}}}"#;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            ("/", LATEST),
            ("/", r#"{"data":{"variableUpsert":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"variableDelete":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"variableUpsert":null}}"#),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        let value = Secret::new("re_live_value".into());
        railway
            .set_provider_variable(&token, "", "p1/s1", "RESEND_API_KEY", &value)
            .await
            .expect("set");
        railway
            .delete_provider_variable(&token, "", "p1/s1", "RESEND_API_KEY")
            .await
            .expect("delete");
        assert!(
            railway
                .set_provider_variable(&token, "", "p1/s1", "RESEND_API_KEY", &value)
                .await
                .is_err(),
            "a refusal is reported, not taken as saved"
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_resources_redeploy_restart_roll_back_and_deploy_the_newest_commit() {
        use crate::application::ports::providers::{ProviderAction, ProviderAdapter};
        const LATEST: &str = r#"{"data":{"deployments":{"edges":[{"node":{"id":"d2","status":"CRASHED","url":null,"staticUrl":null,"environmentId":"e1"}}]}}}"#;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            ("/", LATEST),
            ("/", r#"{"data":{"deploymentRedeploy":{"id":"d3"}}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"deploymentRestart":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"deploymentRollback":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"serviceInstanceDeploy":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"serviceInstanceUpdate":true}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"deploymentRestart":false}}"#),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        for action in [
            ProviderAction::Redeploy,
            ProviderAction::Restart,
            ProviderAction::Rollback {
                deployment_id: "d1",
            },
            ProviderAction::DeployLatest,
            ProviderAction::Sleep { on: true },
        ] {
            let done = railway
                .provider_action(&token, "", "p1/s1", &action)
                .await
                .expect("acted");
            assert!(!done.is_empty());
        }
        assert!(
            railway
                .provider_action(&token, "", "p1/s1", &ProviderAction::Restart)
                .await
                .is_err(),
            "a refusal is reported, not taken as done"
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_folders_upload_like_railway_up_and_settings_change() {
        use crate::application::ports::providers::{
            ProviderAction, ProviderAdapter, ServiceConfig,
        };
        const LATEST: &str = r#"{"data":{"deployments":{"edges":[{"node":{"id":"d2","status":"SUCCESS","url":null,"staticUrl":null,"environmentId":"e1"}}]}}}"#;
        let folder = tempfile::tempdir().expect("folder");
        std::fs::write(folder.path().join("server.js"), "console.log(1)").expect("file");
        std::fs::write(folder.path().join(".gitignore"), "secret.env\n").expect("ignore");
        std::fs::write(folder.path().join("secret.env"), "TOKEN=x").expect("ignored file");
        std::fs::create_dir_all(folder.path().join("node_modules/x")).expect("modules");
        std::fs::write(folder.path().join("node_modules/x/i.js"), "1").expect("module");
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            ("/", LATEST),
            (
                "/project/p1/environment/e1/up?serviceId=s1&message=fix|content-type: application/gzip",
                r#"{"deploymentId":"dep-12345678","url":"","logsUrl":"","deploymentDomain":""}"#,
            ),
            ("/", LATEST),
            ("/", r#"{"data":{"serviceInstanceUpdate":true}}"#),
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"serviceDomainCreate":{"id":"sd1","domain":"x.up.railway.app"}}}"#,
            ),
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"domains":{"serviceDomains":[{"id":"sd1","domain":"x.up.railway.app"}],"customDomains":[]}}}"#,
            ),
            ("/", r#"{"data":{"serviceDomainDelete":true}}"#),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        let done = railway
            .provider_upload(&token, "", "p1/s1", folder.path(), Some("fix"))
            .await
            .expect("uploaded");
        assert!(done.contains("dep-1234"));
        let config = ServiceConfig {
            start_command: Some("node server.js".into()),
            replicas: Some(2),
            ..ServiceConfig::default()
        };
        let done = railway
            .provider_action(&token, "", "p1/s1", &ProviderAction::Configure(&config))
            .await
            .expect("configured");
        assert!(done.contains("start command and the number of copies"));
        railway
            .provider_action(
                &token,
                "",
                "p1/s1",
                &ProviderAction::AddDomain {
                    domain: None,
                    port: None,
                },
            )
            .await
            .expect("domain added");
        railway
            .provider_action(
                &token,
                "",
                "p1/s1",
                &ProviderAction::RemoveDomain {
                    domain: "x.up.railway.app",
                },
            )
            .await
            .expect("domain removed");
        task.await.expect("fixture");
        // What was packed: the code, not ignored files, git or node_modules.
        let packed = super::super::railway::pack_for_test(folder.path()).expect("packed");
        assert_eq!(
            packed,
            vec![".gitignore".to_owned(), "server.js".to_owned()]
        );
    }

    #[tokio::test]
    async fn railway_creates_services_and_databases_and_manages_disks_and_endpoints() {
        use crate::application::ports::providers::{
            ProviderAction, ProviderAdapter, ProviderCreate,
        };
        const LATEST: &str = r#"{"data":{"deployments":{"edges":[{"node":{"id":"d2","status":"SUCCESS","url":null,"staticUrl":null,"environmentId":"e1"}}]}}}"#;
        const TEMPLATE: &str = r#"{"data":{"template":{"id":"t1","name":"PostgreSQL","serializedConfig":{"services":{"a":{"name":"Postgres","variables":{"PGDATA":{"defaultValue":"/data"},"POSTGRES_PASSWORD":{"defaultValue":"","isOptional":false}}}}}}}}"#;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            ("/", LATEST),
            (
                "/",
                r#"{"data":{"serviceCreate":{"id":"s9","name":"worker"}}}"#,
            ),
            ("/", LATEST),
            ("/", TEMPLATE),
            ("/", LATEST),
            ("/", TEMPLATE),
            (
                "/",
                r#"{"data":{"project":{"services":{"edges":[{"node":{"id":"s1"}}]}}}}"#,
            ),
            (
                "/",
                r#"{"data":{"templateDeployV2":{"projectId":"p1","workflowId":"w1"}}}"#,
            ),
            (
                "/",
                r#"{"data":{"project":{"services":{"edges":[{"node":{"id":"s1"}},{"node":{"id":"pg"}}]}}}}"#,
            ),
            ("/", LATEST),
            ("/", r#"{"data":{"volumeCreate":{"id":"v1"}}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"tcpProxyCreate":{"id":"x1"}}}"#),
            ("/", LATEST),
            ("/", r#"{"data":{"privateNetworks":[{"publicId":"n1"}]}}"#),
            (
                "/",
                r#"{"data":{"privateNetworkEndpoint":{"publicId":"pe1"}}}"#,
            ),
            ("/", r#"{"data":{"privateNetworkEndpointRename":true}}"#),
        ])
        .await;
        let railway = super::super::Railway::new(http);
        let (created, _) = railway
            .provider_create(
                &token,
                "",
                "p1/s1",
                &ProviderCreate::Service {
                    name: "worker",
                    repo: Some("me/app"),
                    branch: None,
                    image: None,
                },
            )
            .await
            .expect("service");
        assert_eq!(created, vec!["p1/s9".to_owned()]);
        let none = std::collections::BTreeMap::new();
        let missing = railway
            .provider_create(
                &token,
                "",
                "p1/s1",
                &ProviderCreate::Template {
                    code: "postgres",
                    variables: &none,
                },
            )
            .await
            .expect_err("a required value is missing");
        assert!(format!("{missing:?}").contains("POSTGRES_PASSWORD"));
        let values = std::collections::BTreeMap::from([(
            "POSTGRES_PASSWORD".to_owned(),
            "generated".to_owned(),
        )]);
        let (created, _) = railway
            .provider_create(
                &token,
                "",
                "p1/s1",
                &ProviderCreate::Template {
                    code: "postgres",
                    variables: &values,
                },
            )
            .await
            .expect("database");
        assert_eq!(created, vec!["p1/pg".to_owned()]);
        for action in [
            ProviderAction::AddVolume {
                mount_path: "/data",
            },
            ProviderAction::AddTcpProxy { port: 5432 },
            ProviderAction::RenamePrivateHost { name: "api" },
        ] {
            railway
                .provider_action(&token, "", "p1/s1", &action)
                .await
                .expect("acted");
        }
        assert!(
            railway
                .provider_action(
                    &token,
                    "",
                    "p1/s1",
                    &ProviderAction::AddVolume { mount_path: "data" }
                )
                .await
                .is_err(),
            "a relative mount path is refused"
        );
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn railway_deployment_lines_put_the_build_first_and_redact() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture-token".into());
        let (http, task) = response_fixture(vec![
            (
                "/",
                r#"{"data":{"buildLogs":[{"timestamp":"2026-11-01T10:00:00Z","message":"npm ci","severity":"info"}]}}"#,
            ),
            (
                "/",
                r#"{"data":{"deploymentLogs":[{"timestamp":"2026-11-01T10:01:00Z","message":"token fixture-token","severity":"info"}]}}"#,
            ),
        ])
        .await;
        let lines = super::super::Railway::new(http)
            .provider_deployment_logs(&token, "", "p1/s1", "d1")
            .await
            .expect("lines");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].source, "build");
        assert_eq!(lines[0].message, "npm ci");
        assert!(!lines[1].message.contains("fixture-token"));
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn cloudflare_first_and_existing_worker_releases_skip_empty_asset_sessions() {
        use crate::application::ports::providers::{Artifact, ProviderAdapter};
        use crate::domain::model::{ResourceKind, ResourcePlan};
        use std::collections::VecDeque;
        use std::sync::Mutex;
        for (existing, cron_mode) in [
            (false, 0),
            (true, 0),
            (true, 1),
            (true, 2),
            (true, 3),
            (true, 4),
            (false, 5),
        ] {
            let script = if existing {
                "existing-name"
            } else {
                "new-name"
            };
            let root = format!("/accounts/ours/workers/scripts/{script}");
            let mut expected = VecDeque::new();
            if existing {
                expected.push_back((
                    format!("GET {root}/schedules"),
                    200,
                    r#"{"result":{"schedules":[{"cron":"0 1 * * *"}]}}"#,
                ));
                expected.push_back((
                    format!("POST {root}/versions"),
                    200,
                    r#"{"result":{"id":"version-new"}}"#,
                ));
                expected.push_back((format!("POST {root}/deployments"), 200, r#"{"result":{}}"#));
            } else {
                expected.push_back((format!("GET {root}/settings"), 404, "{}"));
                // A script PUT answers with the script's name as `id`, not its version id.
                expected.push_back((
                    format!("PUT {root}"),
                    200,
                    r#"{"result":{"id":"new-name"}}"#,
                ));
                expected.push_back((
                    format!("GET {root}/deployments?per_page=1"),
                    200,
                    r#"{"result":{"deployments":[{"versions":[{"version_id":"version-first"}]}]}}"#,
                ));
            }
            // The account's workers.dev subdomain is checked (and set up if
            // missing) before the Worker is put on it.
            if !existing && cron_mode == 0 {
                // A new account: none yet, so one is made, named after the Worker.
                expected.push_back(("GET /accounts/ours/workers/subdomain".into(), 404, "{}"));
                expected.push_back((
                    "PUT /accounts/ours/workers/subdomain".into(),
                    200,
                    r#"{"result":{"subdomain":"new-name"}}"#,
                ));
            } else {
                expected.push_back((
                    "GET /accounts/ours/workers/subdomain".into(),
                    200,
                    r#"{"result":{"subdomain":"personal"}}"#,
                ));
            }
            expected.push_back((format!("POST {root}/subdomain"), 200, r#"{"result":{}}"#));
            if cron_mode != 5 {
                expected.push_back((
                    "GET /accounts/ours/workers/subdomain".into(),
                    200,
                    r#"{"result":{"subdomain":"personal"}}"#,
                ));
            }
            if cron_mode > 0 {
                expected.push_back((
                    format!("PUT {root}/schedules"),
                    if matches!(cron_mode, 3 | 4) { 500 } else { 200 },
                    r#"{"result":{}}"#,
                ));
            }
            if !matches!(cron_mode, 3 | 4) {
                expected.push_back((format!("POST {root}/deployments"), 200, r#"{"result":{}}"#));
            }
            expected.push_back((
                format!("PUT {root}/schedules"),
                if cron_mode == 4 { 500 } else { 200 },
                r#"{"result":{}}"#,
            ));
            if cron_mode == 5 {
                expected.push_back((format!("GET {root}/settings"), 200, r#"{"result":{}}"#));
            }
            let expected = Arc::new(Mutex::new(expected));
            let requests = expected.clone();
            let schedule_writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let router = axum::Router::new().fallback(move |request: axum::extract::Request| {
                let requests = requests.clone();
                let schedule_writes = schedule_writes.clone();
                async move {
                    let actual = format!("{} {}", request.method(), request.uri());
                    let (wanted, status, body) = requests
                        .lock()
                        .expect("fixture lock")
                        .pop_front()
                        .expect("unexpected provider call");
                    assert_eq!(actual, wanted);
                    let is_multipart = request.headers().get("content-type").is_some_and(|v| {
                        v.to_str().expect("content type").starts_with("multipart/")
                    });
                    let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                        .await
                        .expect("bounded body");
                    if actual.starts_with("PUT ") && actual.ends_with("/schedules") {
                        let data: serde_json::Value =
                            serde_json::from_slice(&bytes).expect("schedule payload");
                        let write_index =
                            schedule_writes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let wanted = if matches!(cron_mode, 3 | 4) && write_index == 1 {
                            serde_json::json!([{"cron":"0 1 * * *"}])
                        } else if cron_mode == 2 || (!existing && cron_mode == 0) {
                            serde_json::json!([])
                        } else if matches!(cron_mode, 1 | 3 | 4 | 5) {
                            serde_json::json!([{"cron":"*/5 * * * *"}])
                        } else {
                            serde_json::json!([{"cron":"0 1 * * *"}])
                        };
                        assert_eq!(data, wanted);
                    }
                    if is_multipart {
                        let text = String::from_utf8(bytes.to_vec()).expect("UTF8 fixture");
                        assert!(text.contains("\"main_module\":\"index.mjs\""));
                        assert!(!text.contains("\"assets\""));
                        assert!(!text.contains("ASSETS"));
                        assert!(text.contains("export default"));
                    }
                    (
                        axum::http::StatusCode::from_u16(status).expect("status"),
                        body,
                    )
                }
            });
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
            let address = listener.local_addr().expect("address");
            let shutdown = tokio_util::sync::CancellationToken::new();
            let stop = shutdown.clone();
            let server = tokio::spawn(async move {
                axum::serve(listener, router)
                    .with_graceful_shutdown(stop.cancelled_owned())
                    .await
                    .expect("fixture server");
            });
            let http = ProviderHttp {
                keeper: None,
                tools: None,
                client: Client::builder()
                    .no_proxy()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .expect("client"),
                history: Arc::new(crate::network_history::NetworkHistory::default()),
                calls: Arc::new(Semaphore::new(1)),
                test_base: Some(Url::parse(&format!("http://{address}/")).expect("url")),
                destinations: Arc::default(),
            };
            let adapter = super::super::cloudflare::Cloudflare::new(http);
            let artifact = Artifact {
                provenance: None,
                assets: vec![],
                worker: Some(b"export default {fetch(){return new Response('ok')}}".to_vec()),
                worker_name: "index.mjs".into(),
                worker_modules: vec![],
                image: None,
                secrets: vec![],
            };
            let resource = ResourcePlan {
                schedules: match cron_mode {
                    1 | 3 | 4 | 5 => Some(vec!["*/5 * * * *".into()]),
                    2 => Some(vec![]),
                    _ => None,
                },
                provider_ref: existing.then(|| "worker:existing-name".into()),
                name: "new-name".into(),
                kind: if cron_mode == 5 {
                    ResourceKind::Job
                } else {
                    ResourceKind::Web
                },
                provider: Provider::Cloudflare,
                account_id: "local".into(),
                region: None,
                depends_on: vec![],
            };
            let result = adapter
                .deploy(
                    &Secret::new("fixture-token".into()),
                    "ours",
                    &resource,
                    &artifact,
                )
                .await;
            if matches!(cron_mode, 3 | 4) {
                let error = result.expect_err("schedule replacement failed");
                if cron_mode == 4 {
                    assert_eq!(error.code(), "schedule.recovery_failed");
                }
                shutdown.cancel();
                server.await.expect("joined fixture");
                assert!(expected.lock().unwrap().is_empty());
                continue;
            }
            let release = result.expect("release");
            assert!(release.schedules.is_some());
            adapter
                .rollback(&Secret::new("fixture-token".into()), "ours", &release)
                .await
                .expect("rollback restores cron snapshot");
            if cron_mode == 5 {
                adapter
                    .health(&Secret::new("fixture-token".into()), "ours", &release)
                    .await
                    .expect("non-invoking job observation");
            }
            shutdown.cancel();
            server.await.expect("joined fixture");
            assert_eq!(
                release.version,
                if existing {
                    "version-new"
                } else {
                    "version-first"
                }
            );
            assert_eq!(release.reference, format!("worker:{script}"));
            assert_eq!(
                release.url,
                (cron_mode != 5).then(|| format!("https://{script}.personal.workers.dev"))
            );
            assert!(expected.lock().expect("fixture lock").is_empty());
        }
    }

    #[tokio::test]
    async fn cloudflare_domains_wait_for_dns_then_attach_and_observe_certificates() {
        use crate::application::ports::providers::ProviderAdapter;
        use crate::domain::domains::DomainReadiness;
        let pending_zone = r#"{"result":[{"id":"zone","name":"example.com","account":{"id":"ours"},"status":"pending","name_servers":["ada.ns.cloudflare.com","bob.ns.cloudflare.com"],"original_registrar":"Example Registrar"}]}"#;
        let active_zone = r#"{"result":[{"id":"zone","name":"example.com","account":{"id":"ours"},"status":"active","name_servers":["ada.ns.cloudflare.com","bob.ns.cloudflare.com"]}]}"#;
        let attached = r#"{"result":{"id":"domain","hostname":"app.example.com","service":"app","zone_id":"zone","enabled":true,"cert_id":"certificate"}}"#;
        let active_cert = r#"{"result":[{"status":"active","certificates":[{"status":"active","hosts":["*.example.com"],"expires_on":"2099-01-01T00:00:00Z"}]}]}"#;
        let token = Secret::new("fixture".into());
        let (http, task) = response_fixture(vec![("/zones?", pending_zone)]).await;
        let observation = super::super::cloudflare::Cloudflare::new(http)
            .configure_domain(
                &token,
                "ours",
                "worker:app",
                "app.example.com",
                "example.com",
                false,
            )
            .await
            .expect("pending zone");
        assert_eq!(observation.dns, DomainReadiness::Pending);
        assert_eq!(observation.https, DomainReadiness::Unknown);
        assert!(observation.provider_domain_id.is_none());
        assert_eq!(observation.nameservers.len(), 2);
        assert_eq!(observation.registrar.as_deref(), Some("Example Registrar"));
        task.await.expect("fixture");
        for (certs, expected) in [
            (active_cert, DomainReadiness::Active),
            (
                r#"{"result":[{"status":"active","certificates":[{"status":"active","hosts":["*.example.com"],"expires_on":"2000-01-01T00:00:00Z"}]}]}"#,
                DomainReadiness::Pending,
            ),
            (
                r#"{"result":[{"status":"active","certificates":[{"status":"active","hosts":["other.example.com"],"expires_on":"2099-01-01T00:00:00Z"}]}]}"#,
                DomainReadiness::Pending,
            ),
            (
                r#"{"success":false,"errors":[{"message":"sensitive failure"}]}"#,
                DomainReadiness::Unknown,
            ),
        ] {
            let (http, task) = response_fixture(vec![
                ("/zones?", active_zone),
                (
                    "/workers/domains?hostname=app.example.com",
                    r#"{"result":[]}"#,
                ),
                ("PUT /accounts/ours/workers/domains", attached),
                ("/ssl/certificate_packs?", certs),
            ])
            .await;
            let observation = super::super::cloudflare::Cloudflare::new(http)
                .configure_domain(
                    &token,
                    "ours",
                    "worker:app",
                    "app.example.com",
                    "example.com",
                    false,
                )
                .await
                .expect("attached domain");
            assert_eq!(observation.dns, DomainReadiness::Active);
            assert_eq!(observation.https, expected);
            assert_eq!(observation.provider_domain_id.as_deref(), Some("domain"));
            assert!(!format!("{observation:?}").contains("sensitive failure"));
            task.await.expect("fixture");
        }
    }

    #[tokio::test]
    async fn domain_ownership_is_checked_before_attachment_or_detachment() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture".into());
        let zone = r#"{"result":[{"id":"zone","name":"example.com","account":{"id":"ours"},"status":"active","name_servers":["ada.ns.cloudflare.com"]}]}"#;
        let other = r#"{"result":[{"id":"domain","hostname":"app.example.com","service":"someone-else","zone_id":"zone","enabled":true}]}"#;
        let (http, task) = response_fixture(vec![
            ("GET /zones?", zone),
            ("GET /accounts/ours/workers/domains?", other),
        ])
        .await;
        let result = super::super::cloudflare::Cloudflare::new(http)
            .configure_domain(
                &token,
                "ours",
                "worker:app",
                "app.example.com",
                "example.com",
                false,
            )
            .await;
        assert!(matches!(result, Err(EngineError::Conflict(_))));
        task.await.expect("fixture");
        let (http, task) =
            response_fixture(vec![("GET /accounts/ours/workers/domains?", other)]).await;
        let result = super::super::cloudflare::Cloudflare::new(http)
            .detach_domain(&token, "ours", "worker:app", None, "app.example.com")
            .await;
        assert!(matches!(result, Err(EngineError::Conflict(_))));
        task.await.expect("fixture");
        let ours = r#"{"result":[{"id":"domain","hostname":"app.example.com","service":"app","zone_id":"zone","enabled":true}]}"#;
        let (http, task) = response_fixture(vec![
            ("GET /accounts/ours/workers/domains?", ours),
            (
                "DELETE /accounts/ours/workers/domains/domain",
                r#"{"result":{}}"#,
            ),
        ])
        .await;
        super::super::cloudflare::Cloudflare::new(http)
            .detach_domain(&token, "ours", "worker:app", None, "app.example.com")
            .await
            .expect("detach after lost response");
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn missing_domain_zone_requires_explicit_creation() {
        use crate::application::ports::providers::ProviderAdapter;
        let token = Secret::new("fixture".into());
        let (http, task) = response_fixture(vec![("GET /zones?", r#"{"result":[]}"#)]).await;
        assert!(
            super::super::cloudflare::Cloudflare::new(http)
                .configure_domain(
                    &token,
                    "ours",
                    "worker:app",
                    "app.example.com",
                    "example.com",
                    false
                )
                .await
                .is_err()
        );
        task.await.expect("fixture");
        let zone = r#"{"result":{"id":"zone","name":"example.com","account":{"id":"ours"},"status":"pending","name_servers":["ada.ns.cloudflare.com"]}}"#;
        let (http, task) = response_fixture(vec![
            ("GET /zones?", r#"{"result":[]}"#),
            ("POST /zones", zone),
        ])
        .await;
        let value = super::super::cloudflare::Cloudflare::new(http)
            .configure_domain(
                &token,
                "ours",
                "worker:app",
                "app.example.com",
                "example.com",
                true,
            )
            .await
            .expect("created");
        assert_eq!(value.dns, crate::domain::domains::DomainReadiness::Pending);
        task.await.expect("fixture");
    }

    #[tokio::test]
    async fn cloudflare_verifies_read_access_without_requiring_token_administration() {
        use crate::application::ports::providers::ProviderAdapter;
        for account_owned in [false, true] {
            let mut replies = vec![];
            if account_owned {
                replies.push(("GET /user/tokens/verify", r#"{"success":false}"#));
                replies.push((
                    "GET /accounts/ours/tokens/verify",
                    r#"{"result":{"id":"token","status":"active"}}"#,
                ));
            } else {
                replies.push((
                    "GET /user/tokens/verify",
                    r#"{"result":{"id":"token","status":"active"}}"#,
                ));
            }
            replies.push(("GET /accounts/ours/workers/scripts", r#"{"result":[]}"#));
            replies.push((
                if account_owned {
                    "GET /accounts/ours/tokens/token"
                } else {
                    "GET /user/tokens/token"
                },
                r#"{"success":false}"#,
            ));
            let (http, task) = response_fixture(replies).await;
            let scopes = super::super::cloudflare::Cloudflare::new(http)
                .verify(&Secret::new("fixture".into()), "ours")
                .await
                .expect("read verified");
            assert!(scopes.iter().any(|scope| scope.contains("unverified")));
            assert!(!scopes.iter().any(|scope| scope == "Workers Scripts Edit"));
            task.await.expect("fixture");
        }
    }

    #[tokio::test]
    async fn fly_pause_disables_startup_before_confirming_stopped_state() {
        use crate::application::ports::providers::ProviderAdapter;
        let initial = r#"{"id":"machine","instance_id":"version-one","state":"started","config":{"image":"registry/app@sha256:abc","env":{"SECRET":"fixture-secret"},"services":[{"internal_port":8080,"autostart":true}],"restart":{"policy":"always"}}}"#;
        let (http, task) = response_fixture(vec![
            ("GET /apps/app/machines ", r#"[{"id":"machine"}]"#),
            ("GET /apps/app/machines/machine ", initial),
            (
                "POST /apps/app/machines/machine ",
                r#"{"id":"machine","state":"stopped"}"#,
            ),
            (
                "GET /apps/app/machines/machine ",
                r#"{"id":"machine","state":"stopped"}"#,
            ),
            (
                "GET /apps/app/machines/machine ",
                r#"{"id":"machine","state":"stopped"}"#,
            ),
        ])
        .await;
        super::super::fly::Fly::new(http)
            .pause(&Secret::new("fixture".into()), "org", "app")
            .await
            .expect("paused");
        task.await.expect("fixture");
    }
}
