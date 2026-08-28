//! GitHub API client: one GraphQL round-trip for the bulk of a repo's
//! facts, plus a few small REST calls for things GraphQL doesn't expose
//! well (languages, contributor count, gists).
//!
//! Every response is deserialized into a typed struct (see below) rather
//! than traversed as raw `serde_json::Value` — a typo in a field path used
//! to fail silently (returning `Value::Null`, indistinguishable from the
//! API genuinely returning null); with typed structs, a field-name
//! mismatch is a deserialization error you see immediately, not a silent
//! `None` three steps downstream. The field names mirror exactly what the
//! hand-written GraphQL query below requests, so this is a mechanical
//! transcription of a known, fixed query shape — not a guess at an
//! external schema. See
//! `ADRs/typed-response-models-over-raw-json-values.md` for the full
//! rationale and the alternatives considered.
//!
//! All requests go through `send_with_retry`, which retries transient
//! network errors, 429s, 5xxs, and rate-limit-flavored 403s with
//! exponential backoff (honoring `Retry-After` when GitHub sends one), and
//! proactively pauses before the next call if `X-RateLimit-Remaining` is
//! nearly exhausted. See `retry.rs` for the underlying policy, which is
//! unit-tested independently of any HTTP I/O.

use crate::retry;
use anyhow::{Context, Result};
use reqwest::header::HeaderMap;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::time::{sleep, Duration};

/// GitHub REST/GraphQL API version pinned via the `X-GitHub-Api-Version`
/// header on every request. Surfaced in `report.json`'s `run_summary` so a
/// reader can tell which API contract produced the data.
pub const GITHUB_API_VERSION: &str = "2022-11-28";

const RATE_LIMIT_FLOOR: i64 = 2;
/// Never auto-pause for longer than this waiting out a rate-limit window;
/// beyond this, proceed and let the call fail normally rather than block a
/// batch run indefinitely on a single link.
const MAX_PROACTIVE_WAIT_SECS: i64 = 900;

pub struct GitHub {
    client: Client,
    token: Option<String>,
    base_url: String,
    max_retries: u32,
}

impl GitHub {
    pub fn new(client: Client, token: Option<String>, max_retries: u32) -> Self {
        Self {
            client,
            token,
            base_url: "https://api.github.com".to_string(),
            max_retries: max_retries.max(1),
        }
    }

    /// Overrides the API base URL. Used two ways: (1) `main.rs`'s hidden,
    /// test-only `--github-base-url` flag (default:
    /// `https://api.github.com`, i.e. a no-op for real runs), which lets
    /// `tests/e2e.rs` point the actual compiled binary at a local mock
    /// server; (2) directly by the `wiremock`-based HTTP-boundary and
    /// orchestration test modules in this crate. No ADR covers this — it's
    /// a testability hook, not an architecture decision.
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req = req
            .header(
                "User-Agent",
                concat!("ghlinks-collector/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION);
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req
    }

    /// Sends a request, retrying per `retry::should_retry` until success,
    /// a non-retryable failure, or `max_retries` attempts are used up.
    /// Returns the final status/body/headers regardless of success —
    /// callers decide what a given status means for their endpoint (e.g.
    /// `repo_exists` treats 404 as a valid, non-error answer).
    async fn send_with_retry(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<&Value>,
    ) -> Result<(StatusCode, String, HeaderMap)> {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let mut req = self.client.request(method.clone(), url);
            if let Some(b) = body {
                req = req.json(b);
            }
            req = self.auth(req);

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt >= self.max_retries {
                        return Err(e)
                            .context(format!("request to {url} failed after {attempt} attempt(s)"));
                    }
                    let delay = retry::backoff_delay(attempt);
                    eprintln!(
                        "  transient error on {url} (attempt {attempt}/{}): {e}; retrying in {delay:?}",
                        self.max_retries
                    );
                    sleep(delay).await;
                    continue;
                }
            };

            let status = resp.status();
            let headers = resp.headers().clone();

            if retry::should_retry(status, &headers, attempt, self.max_retries) {
                let delay = retry::retry_delay(&headers, attempt);
                eprintln!(
                    "  retryable HTTP {status} on {url} (attempt {attempt}/{}); retrying in {delay:?}",
                    self.max_retries
                );
                sleep(delay).await;
                continue;
            }

            self.throttle_if_low(&headers).await;

            let text = response_text(resp).await?;
            return Ok((status, text, headers));
        }
    }

    async fn throttle_if_low(&self, headers: &HeaderMap) {
        let remaining: Option<i64> = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        let reset_epoch: Option<i64> = headers
            .get("x-ratelimit-reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok());
        if let Some(wait_secs) = retry::proactive_wait_secs(
            remaining,
            reset_epoch,
            RATE_LIMIT_FLOOR,
            MAX_PROACTIVE_WAIT_SECS,
        ) {
            eprintln!("  GitHub rate limit nearly exhausted; pausing {wait_secs}s until reset");
            sleep(Duration::from_secs(wait_secs as u64)).await;
        }
    }

    pub async fn graphql_repo(&self, owner: &str, name: &str) -> Result<RepositoryNode> {
        let query = r#"
        query($owner:String!, $name:String!) {
          repository(owner:$owner, name:$name) {
            description
            homepageUrl
            licenseInfo { key name }
            stargazerCount
            forkCount
            watchers { totalCount }
            openIssues: issues(states: OPEN) { totalCount }
            closedIssues: issues(states: CLOSED) { totalCount }
            primaryLanguage { name }
            createdAt
            pushedAt
            defaultBranchRef {
              name
              target {
                ... on Commit { history { totalCount } }
              }
            }
            repositoryTopics(first: 25) { nodes { topic { name } } }
            releases { totalCount }
          }
        }"#;
        let body = json!({
            "query": query,
            "variables": { "owner": owner, "name": name }
        });
        let url = format!("{}/graphql", self.base_url);
        let (status, text, _headers) = self
            .send_with_retry(reqwest::Method::POST, &url, Some(&body))
            .await
            .context("graphql request failed")?;
        if !status.is_success() {
            anyhow::bail!("GraphQL HTTP {status}: {text}");
        }
        let envelope: GraphQlEnvelope<RepoQueryData> =
            serde_json::from_str(&text).context("parsing graphql repository json")?;
        if let Some(errors) = envelope.errors {
            let joined = errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            anyhow::bail!("GraphQL returned errors: {joined}");
        }
        envelope
            .data
            .and_then(|d| d.repository)
            .ok_or_else(|| anyhow::anyhow!("repository {owner}/{name} not found or inaccessible"))
    }

    pub async fn languages(&self, owner: &str, repo: &str) -> Result<BTreeMap<String, i64>> {
        let url = format!("{}/repos/{owner}/{repo}/languages", self.base_url);
        let (status, text, _headers) = self
            .send_with_retry(reqwest::Method::GET, &url, None)
            .await?;
        if !status.is_success() {
            anyhow::bail!("languages HTTP {status}: {text}");
        }
        serde_json::from_str(&text).context("parsing languages json")
    }

    pub async fn contributors_count(&self, owner: &str, repo: &str) -> Result<Option<i64>> {
        let url = format!(
            "{}/repos/{owner}/{repo}/contributors?per_page=1&anon=true",
            self.base_url
        );
        let (status, text, headers) = self
            .send_with_retry(reqwest::Method::GET, &url, None)
            .await?;
        if !status.is_success() {
            anyhow::bail!("contributors HTTP {status}: {text}");
        }
        // Prefer the `Link: rel="last"` page number when present — GitHub
        // caps this listing at one contributor per page via `per_page=1`,
        // so the last page number IS the contributor count. Falls back to
        // counting the returned array only if there's no Link header
        // (i.e. the whole result fit on one page).
        if let Some(link) = headers.get("link").and_then(|v| v.to_str().ok()) {
            if let Some(last) = parse_last_page(link) {
                return Ok(Some(last));
            }
        }
        let body: Value = serde_json::from_str(&text).context("parsing contributors json")?;
        Ok(body.as_array().map(|a| a.len() as i64))
    }

    pub async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        let url = format!("{}/repos/{owner}/{repo}", self.base_url);
        let (status, _text, _headers) = self
            .send_with_retry(reqwest::Method::GET, &url, None)
            .await?;
        if status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !status.is_success() {
            anyhow::bail!("repository existence check HTTP {status}");
        }
        Ok(true)
    }

    pub async fn releases(&self, owner: &str, name: &str) -> Result<Vec<ReleaseNode>> {
        let query = r#"
        query($owner:String!, $name:String!, $after:String) {
          repository(owner:$owner, name:$name) {
            releases(first: 100, after: $after, orderBy: {field: CREATED_AT, direction: DESC}) {
              nodes { tagName name publishedAt }
              pageInfo { hasNextPage endCursor }
            }
          }
        }"#;
        let url = format!("{}/graphql", self.base_url);
        let mut after: Option<String> = None;
        let mut releases = Vec::new();
        loop {
            let body = json!({
                "query": query,
                "variables": { "owner": owner, "name": name, "after": after }
            });
            let (status, text, _headers) = self
                .send_with_retry(reqwest::Method::POST, &url, Some(&body))
                .await?;
            if !status.is_success() {
                anyhow::bail!("releases GraphQL HTTP {status}: {text}");
            }
            let envelope: GraphQlEnvelope<ReleasesQueryData> =
                serde_json::from_str(&text).context("parsing releases graphql json")?;
            if let Some(errors) = envelope.errors {
                let joined = errors
                    .into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join("; ");
                anyhow::bail!("releases GraphQL returned errors: {joined}");
            }
            let connection = envelope
                .data
                .and_then(|d| d.repository)
                .and_then(|r| r.releases)
                .ok_or_else(|| anyhow::anyhow!("releases connection missing from response"))?;
            releases.extend(connection.nodes);
            if !connection.page_info.has_next_page {
                break;
            }
            after = connection.page_info.end_cursor;
            if after.is_none() {
                anyhow::bail!("releases page claimed a next page but returned no cursor");
            }
        }
        Ok(releases)
    }

    pub async fn gist(&self, gist_id: &str) -> Result<GistResponse> {
        let url = format!("{}/gists/{gist_id}", self.base_url);
        let (status, text, _headers) = self
            .send_with_retry(reqwest::Method::GET, &url, None)
            .await?;
        if !status.is_success() {
            anyhow::bail!("Gist HTTP {status}: {text}");
        }
        serde_json::from_str(&text).context("parsing gist json")
    }
}

async fn response_text(resp: reqwest::Response) -> Result<String> {
    resp.text().await.context("reading response body")
}

/// Parses the RFC 5988 `Link` header GitHub sends on paginated REST
/// endpoints and returns the `page` query-param value from the
/// `rel="last"` entry, if present.
fn parse_last_page(link_header: &str) -> Option<i64> {
    for part in link_header.split(',') {
        if part.contains(r#"rel="last""#) {
            let url_part = part.split(';').next()?.trim().trim_start_matches('<').trim_end_matches('>');
            let url = url::Url::parse(url_part).ok()?;
            for (k, v) in url.query_pairs() {
                if k == "page" {
                    return v.parse().ok();
                }
            }
        }
    }
    None
}

// ---- GraphQL response types -----------------------------------------
//
// Field names below use `#[serde(rename_all = "camelCase")]`, which maps
// Rust's `snake_case` to the exact camelCase field/alias names in the
// query above (e.g. `open_issues` -> `openIssues`, matching the query's
// `openIssues:` alias). This is a direct transcription of that query, not
// a guess at GitHub's schema.

#[derive(Deserialize, Debug)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize, Debug)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize, Debug)]
struct RepoQueryData {
    repository: Option<RepositoryNode>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryNode {
    pub description: Option<String>,
    pub homepage_url: Option<String>,
    pub license_info: Option<LicenseInfo>,
    pub stargazer_count: Option<i64>,
    pub fork_count: Option<i64>,
    pub watchers: Option<CountWrapper>,
    pub open_issues: Option<CountWrapper>,
    pub closed_issues: Option<CountWrapper>,
    pub primary_language: Option<NameWrapper>,
    pub created_at: Option<String>,
    pub pushed_at: Option<String>,
    pub default_branch_ref: Option<DefaultBranchRef>,
    pub repository_topics: Option<TopicsConnection>,
    pub releases: Option<CountWrapper>,
}

#[derive(Deserialize, Debug, Default)]
pub struct LicenseInfo {
    pub key: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CountWrapper {
    pub total_count: Option<i64>,
}

#[derive(Deserialize, Debug, Default)]
pub struct NameWrapper {
    pub name: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub struct DefaultBranchRef {
    pub name: Option<String>,
    pub target: Option<CommitTarget>,
}

#[derive(Deserialize, Debug, Default)]
pub struct CommitTarget {
    pub history: Option<CountWrapper>,
}

#[derive(Deserialize, Debug, Default)]
pub struct TopicsConnection {
    #[serde(default)]
    pub nodes: Vec<TopicNode>,
}

#[derive(Deserialize, Debug)]
pub struct TopicNode {
    pub topic: Option<TopicName>,
}

#[derive(Deserialize, Debug)]
pub struct TopicName {
    pub name: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ReleasesQueryData {
    repository: Option<RepositoryReleases>,
}

#[derive(Deserialize, Debug)]
struct RepositoryReleases {
    releases: Option<ReleasesConnection>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct ReleasesConnection {
    #[serde(default)]
    nodes: Vec<ReleaseNode>,
    #[serde(default)]
    page_info: PageInfo,
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNode {
    pub tag_name: Option<String>,
    pub name: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    #[serde(default)]
    has_next_page: bool,
    end_cursor: Option<String>,
}

// ---- REST response types ----------------------------------------------
// GitHub's REST API already uses snake_case, so no rename attribute is
// needed here.

#[derive(Deserialize, Debug, Default)]
pub struct GistResponse {
    pub description: Option<String>,
    pub owner: Option<GistOwner>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments: Option<i64>,
    #[serde(default)]
    pub history: Vec<Value>,
    #[serde(default)]
    pub files: BTreeMap<String, Value>,
}

#[derive(Deserialize, Debug, Default)]
pub struct GistOwner {
    pub login: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_last_page_among_pagination_links() {
        let header = concat!(
            r#"<https://api.github.com/x?page=2>; rel="next", "#,
            r#"<https://api.github.com/x?page=42>; rel="last""#
        );
        assert_eq!(parse_last_page(header), Some(42));
    }

    #[test]
    fn returns_none_for_link_headers_with_no_last_page() {
        let header = r#"<https://api.github.com/x?page=2>; rel="next""#;
        assert_eq!(parse_last_page(header), None);
    }

    #[test]
    fn ignores_malformed_link_headers() {
        assert_eq!(parse_last_page("not a link header"), None);
        assert_eq!(parse_last_page(""), None);
    }

    #[test]
    fn deserializes_a_full_graphql_repo_response() {
        let json = r#"{
            "data": {
                "repository": {
                    "description": "desc",
                    "homepageUrl": "https://example.com",
                    "licenseInfo": { "key": "mit", "name": "MIT License" },
                    "stargazerCount": 12,
                    "forkCount": 3,
                    "watchers": { "totalCount": 4 },
                    "openIssues": { "totalCount": 1 },
                    "closedIssues": { "totalCount": 2 },
                    "primaryLanguage": { "name": "Rust" },
                    "createdAt": "2026-01-01T00:00:00Z",
                    "pushedAt": "2026-02-01T00:00:00Z",
                    "defaultBranchRef": {
                        "name": "main",
                        "target": { "history": { "totalCount": 99 } }
                    },
                    "repositoryTopics": { "nodes": [ { "topic": { "name": "cli" } } ] },
                    "releases": { "totalCount": 5 }
                }
            }
        }"#;
        let envelope: GraphQlEnvelope<RepoQueryData> = serde_json::from_str(json).unwrap();
        let repo = envelope.data.unwrap().repository.unwrap();
        assert_eq!(repo.description.as_deref(), Some("desc"));
        assert_eq!(repo.homepage_url.as_deref(), Some("https://example.com"));
        assert_eq!(repo.license_info.unwrap().key.as_deref(), Some("mit"));
        assert_eq!(repo.stargazer_count, Some(12));
        assert_eq!(repo.watchers.unwrap().total_count, Some(4));
        assert_eq!(repo.open_issues.unwrap().total_count, Some(1));
        assert_eq!(repo.closed_issues.unwrap().total_count, Some(2));
        assert_eq!(repo.primary_language.unwrap().name.as_deref(), Some("Rust"));
        assert_eq!(
            repo.default_branch_ref
                .unwrap()
                .target
                .unwrap()
                .history
                .unwrap()
                .total_count,
            Some(99)
        );
        assert_eq!(
            repo.repository_topics.unwrap().nodes[0]
                .topic
                .as_ref()
                .unwrap()
                .name
                .as_deref(),
            Some("cli")
        );
        assert_eq!(repo.releases.unwrap().total_count, Some(5));
    }

    #[test]
    fn graphql_errors_array_is_captured_and_data_is_absent() {
        let json = r#"{"errors":[{"message":"boom"}]}"#;
        let envelope: GraphQlEnvelope<RepoQueryData> = serde_json::from_str(json).unwrap();
        assert!(envelope.data.is_none());
        assert_eq!(envelope.errors.unwrap()[0].message, "boom");
    }

    #[test]
    fn null_repository_deserializes_to_none_not_a_deserialize_error() {
        let json = r#"{"data":{"repository":null}}"#;
        let envelope: GraphQlEnvelope<RepoQueryData> = serde_json::from_str(json).unwrap();
        assert!(envelope.data.unwrap().repository.is_none());
    }

    #[test]
    fn deserializes_a_paged_releases_connection() {
        let json = r#"{
            "data": {
                "repository": {
                    "releases": {
                        "nodes": [
                            {"tagName":"v2","name":"Two","publishedAt":"2026-02-01T00:00:00Z"},
                            {"tagName":"v1","name":null,"publishedAt":"2026-01-01T00:00:00Z"}
                        ],
                        "pageInfo": {"hasNextPage": true, "endCursor": "abc"}
                    }
                }
            }
        }"#;
        let envelope: GraphQlEnvelope<ReleasesQueryData> = serde_json::from_str(json).unwrap();
        let connection = envelope.data.unwrap().repository.unwrap().releases.unwrap();
        assert_eq!(connection.nodes.len(), 2);
        assert_eq!(connection.nodes[0].tag_name.as_deref(), Some("v2"));
        assert_eq!(connection.nodes[1].name, None);
        assert!(connection.page_info.has_next_page);
        assert_eq!(connection.page_info.end_cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn releases_connection_with_no_further_pages() {
        let json = r#"{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}"#;
        let connection: ReleasesConnection = serde_json::from_str(json).unwrap();
        assert!(connection.nodes.is_empty());
        assert!(!connection.page_info.has_next_page);
    }

    #[test]
    fn deserializes_gist_response_and_ignores_unmodeled_file_fields() {
        let json = r#"{
            "description": "a gist",
            "owner": {"login": "octocat"},
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-02T00:00:00Z",
            "comments": 3,
            "history": [{"version":"a"},{"version":"b"}],
            "files": {"foo.py": {"filename":"foo.py","size":10}}
        }"#;
        let gist: GistResponse = serde_json::from_str(json).unwrap();
        assert_eq!(gist.owner.unwrap().login.as_deref(), Some("octocat"));
        assert_eq!(gist.comments, Some(3));
        assert_eq!(gist.history.len(), 2);
        assert!(gist.files.contains_key("foo.py"));
    }

    #[test]
    fn gist_response_tolerates_missing_optional_fields() {
        let json = r#"{"description": null}"#;
        let gist: GistResponse = serde_json::from_str(json).unwrap();
        assert!(gist.owner.is_none());
        assert!(gist.history.is_empty());
        assert!(gist.files.is_empty());
    }
}

/// T-3: HTTP-boundary integration tests. Unlike the parser-only tests
/// above (which prove typed structs deserialize correctly given a JSON
/// string), these run the *actual* `reqwest` client against a local
/// `wiremock` server via `GitHub::with_base_url()` — proving what
/// `ghlinks` actually does at HTTP failure boundaries (status codes,
/// retries, pagination, malformed bodies), rather than assuming it from
/// reading the code. `wiremock` is a dev-dependency only (see Cargo.toml);
/// nothing here runs in the release binary.
///
/// Deliberately scoped to representative behavioral contracts (per the
/// review that motivated this test module), not exhaustive per-endpoint
/// coverage: one test per distinct *behavior*, not one test per endpoint.
#[cfg(test)]
mod http_boundary_tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// Returns queued responses in order, one per received request, so a
    /// single mounted mock can simulate a sequence (e.g. 500, 500, 200)
    /// without wiremock's own call-count matchers.
    struct SequencedResponses(Mutex<VecDeque<ResponseTemplate>>);

    impl SequencedResponses {
        fn new(responses: Vec<ResponseTemplate>) -> Self {
            Self(Mutex::new(responses.into_iter().collect()))
        }
    }

    impl Respond for SequencedResponses {
        fn respond(&self, _req: &Request) -> ResponseTemplate {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| ResponseTemplate::new(500))
        }
    }

    fn client_against(base_url: &str, max_retries: u32) -> GitHub {
        GitHub::new(reqwest::Client::new(), None, max_retries).with_base_url(base_url)
    }

    fn ok_repo_body() -> serde_json::Value {
        json!({
            "data": {
                "repository": {
                    "description": "a test repo",
                    "stargazerCount": 5,
                    "releases": { "totalCount": 1 }
                }
            }
        })
    }

    // -- T-3.1: successful GraphQL round-trip through the real HTTP client --
    #[tokio::test]
    async fn graphql_repo_success_goes_through_real_http_to_a_typed_struct() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_repo_body()))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let repo = gh.graphql_repo("owner", "repo").await.unwrap();

        assert_eq!(repo.description.as_deref(), Some("a test repo"));
        assert_eq!(repo.stargazer_count, Some(5));

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "expected exactly one GraphQL request");
        assert_eq!(requests[0].method.as_str(), "POST");
    }

    // -- T-3.2: HTTP 200 carrying a GraphQL-level errors array must NOT --
    // -- be treated as a successful empty repository.                   --
    #[tokio::test]
    async fn graphql_level_errors_in_a_200_response_become_an_application_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"errors": [{"message": "rate limited"}]})),
            )
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let result = gh.graphql_repo("owner", "repo").await;

        assert!(
            result.is_err(),
            "a 200 wrapping a GraphQL errors array must surface as Err, not an empty Ok"
        );
        assert!(result.unwrap_err().to_string().contains("rate limited"));
    }

    // -- T-3.3: a malformed (non-JSON) 200 body is a parse error, not a --
    // -- silent default/empty value.                                    --
    #[tokio::test]
    async fn malformed_response_body_is_reported_as_a_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let result = gh.graphql_repo("owner", "repo").await;
        assert!(result.is_err(), "malformed body must not parse as a valid response");
    }

    // -- T-3.4a: a plain HTTP failure with no rate-limit signal is --
    // -- reported, not retried (retry.rs already unit-tests the policy; --
    // -- this proves the client actually honors it end-to-end).         --
    #[tokio::test]
    async fn plain_403_is_reported_after_exactly_one_attempt() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let result = gh.repo_exists("owner", "repo").await;

        assert!(result.is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "a plain 403 (no rate-limit signal) must not be retried"
        );
    }

    // -- T-3.4b: repo_exists() treats a 404 as Ok(false), not an error --
    // -- (a not-found repo is a valid, non-exceptional answer for this --
    // -- endpoint specifically).                                       --
    #[tokio::test]
    async fn repo_exists_returns_ok_false_on_404_rather_than_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        assert_eq!(gh.repo_exists("owner", "missing").await.unwrap(), false);
    }

    #[tokio::test]
    async fn repo_exists_returns_ok_true_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        assert_eq!(gh.repo_exists("owner", "repo").await.unwrap(), true);
    }

    // -- T-3.5: transient 500s are retried and an eventual 200 succeeds --
    #[tokio::test]
    async fn two_server_errors_then_success_eventually_succeeds_after_retrying() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(SequencedResponses::new(vec![
                ResponseTemplate::new(500),
                ResponseTemplate::new(500),
                ResponseTemplate::new(200).set_body_json(ok_repo_body()),
            ]))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 5);
        let repo = gh.graphql_repo("owner", "repo").await.unwrap();

        assert_eq!(repo.description.as_deref(), Some("a test repo"));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3, "expected 2 failed attempts + 1 success");
    }

    // -- T-3.5b: retries stop at max_retries and the failure is reported --
    #[tokio::test]
    async fn persistent_server_errors_fail_after_exhausting_max_retries() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let result = gh.graphql_repo("owner", "repo").await;

        assert!(result.is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            3,
            "expected exactly max_retries (3) attempts, no more, no fewer"
        );
    }

    // -- T-3.6: releases() follows pagination via the cursor and --
    // -- preserves CREATED_AT DESC order across pages — this is the --
    // -- executable proof behind #6's release semantics. --
    #[tokio::test]
    async fn releases_follows_pagination_cursor_and_preserves_created_at_desc_order() {
        let server = MockServer::start().await;
        let page1 = json!({
            "data": { "repository": { "releases": {
                "nodes": [
                    {"tagName": "v3", "name": "Three", "publishedAt": "2026-03-01T00:00:00Z"},
                    {"tagName": "v2", "name": "Two", "publishedAt": "2026-02-01T00:00:00Z"}
                ],
                "pageInfo": {"hasNextPage": true, "endCursor": "cursor-1"}
            }}}
        });
        let page2 = json!({
            "data": { "repository": { "releases": {
                "nodes": [
                    {"tagName": "v1", "name": "One", "publishedAt": "2026-01-01T00:00:00Z"}
                ],
                "pageInfo": {"hasNextPage": false, "endCursor": null}
            }}}
        });
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(SequencedResponses::new(vec![
                ResponseTemplate::new(200).set_body_json(page1),
                ResponseTemplate::new(200).set_body_json(page2),
            ]))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let releases = gh.releases("owner", "repo").await.unwrap();

        assert_eq!(
            releases.iter().map(|r| r.tag_name.clone()).collect::<Vec<_>>(),
            vec![Some("v3".into()), Some("v2".into()), Some("v1".into())],
            "release order across pages must remain CREATED_AT DESC, not be re-sorted"
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2, "expected one request per page, cursor-chained");
    }

    // -- T-3.7a: contributors_count() uses the Link header's rel="last" --
    // -- page number when present.                                     --
    #[tokio::test]
    async fn contributors_count_reads_the_last_page_number_from_the_link_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/contributors"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!([{"login": "a"}]))
                    .insert_header(
                        "link",
                        format!(
                            r#"<{u}?page=2>; rel="next", <{u}?page=14>; rel="last""#,
                            u = format!("{}/repos/owner/repo/contributors", server.uri())
                        )
                        .as_str(),
                    ),
            )
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        assert_eq!(gh.contributors_count("owner", "repo").await.unwrap(), Some(14));
    }

    // -- T-3.7b: with no Link header (whole result fit on one page), --
    // -- falls back to counting the returned array.                  --
    #[tokio::test]
    async fn contributors_count_falls_back_to_array_length_with_no_link_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/owner/repo/contributors"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"login": "a"}])))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        assert_eq!(gh.contributors_count("owner", "repo").await.unwrap(), Some(1));
    }

    // -- T-3.8: gist() end-to-end through the real HTTP client --
    #[tokio::test]
    async fn gist_success_goes_through_real_http_to_a_typed_struct() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gists/abc123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "description": "a test gist",
                "owner": {"login": "octocat"},
                "comments": 2,
                "history": [],
                "files": {}
            })))
            .mount(&server)
            .await;

        let gh = client_against(&server.uri(), 3);
        let gist = gh.gist("abc123").await.unwrap();
        assert_eq!(gist.description.as_deref(), Some("a test gist"));
        assert_eq!(gist.owner.unwrap().login.as_deref(), Some("octocat"));
    }
}