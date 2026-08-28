//! Discovery of external mentions using APIs that require no key/auth.
//! This intentionally does NOT attempt sentiment or stance classification —
//! it only collects raw signals (title, score, comment count). Reading tone
//! from these is left to a human or an LLM working from this output.
//!
//! Reddit was removed from this module (was: unauthenticated
//! `reddit.com/search.json`, which stopped working entirely in 2026).
//! Reddit mention-discovery now happens downstream, during the LLM
//! synthesis pass over this tool's output — see
//! `ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md`.
//!
//! The Hacker News call below goes through the same retry/backoff policy
//! as the GitHub client (see `retry.rs`), sharing one implementation
//! rather than reinventing it per source.
//!
//! `hacker_news()` takes its base URL as a parameter (default:
//! `HN_DEFAULT_BASE_URL`) rather than hardcoding it, mirroring
//! `github.rs::GitHub::with_base_url()` — the same seam, in function-
//! parameter form since this module has no client struct to hang a
//! builder method off of. `main.rs`'s hidden `--hn-base-url` flag is the
//! production call site's only consumer of this; it exists so
//! `http_boundary_tests` below and `main.rs`'s orchestration/e2e tests can
//! point this call at a local mock server, the same way the GitHub side
//! already could.
//!
//! Unlike `github.rs`, the Algolia response here is read via raw
//! `serde_json::Value` lookups rather than typed structs (see
//! `ADRs/typed-response-models-over-raw-json-values.md`, which is scoped
//! to GitHub's GraphQL/REST responses). This is a deliberate, narrower
//! risk tradeoff, not an oversight: Hacker News mentions are supplementary,
//! best-effort signal — the README already treats `external_mentions` as
//! a floor, not a ceiling — and a missing field here degrades visibly
//! (`.unwrap_or("(untitled)")`) rather than silently producing a wrong
//! fact the way an untyped miss on `stargazers_count` or `license_key`
//! would. If this endpoint's response shape grows more fields this module
//! actually depends on, revisit typed structs here too.

use crate::model::ExternalMention;
use crate::retry;
use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;
use tokio::time::sleep;

/// Hacker News' Algolia Search API has no documented hard rate limit for
/// this kind of light, spaced-out usage, but it can still return a
/// transient 5xx or 429 under load — same retry budget as GitHub calls.
const HN_MAX_RETRIES: u32 = 3;

/// The real Hacker News Algolia Search API. Every production call site
/// should use this; `main.rs`'s `--hn-base-url` flag defaults to it, and
/// only overrides it for tests.
pub const HN_DEFAULT_BASE_URL: &str = "https://hn.algolia.com";

/// Hacker News stories linking to the exact URL, via Algolia's HN Search
/// API (`restrictSearchableAttributes=url`). Only stories are searched,
/// not comments — comment-level mentions of a URL are a different, much
/// noisier signal this tool deliberately doesn't chase.
pub async fn hacker_news(
    client: &Client,
    target_url: &str,
    base_url: &str,
) -> Result<Vec<ExternalMention>> {
    let endpoint = format!(
        "{base_url}/api/v1/search?query={}&restrictSearchableAttributes=url&tags=story",
        urlencoding::encode(target_url)
    );

    let mut attempt: u32 = 0;
    let (resp, status) = loop {
        attempt += 1;
        let sent = client.get(&endpoint).send().await;
        let resp = match sent {
            Ok(r) => r,
            Err(e) => {
                if attempt >= HN_MAX_RETRIES {
                    return Err(e).context(format!(
                        "Hacker News request failed after {attempt} attempt(s)"
                    ));
                }
                let delay = retry::backoff_delay(attempt);
                eprintln!(
                    "  transient error on Hacker News search (attempt {attempt}/{HN_MAX_RETRIES}): {e}; retrying in {delay:?}"
                );
                sleep(delay).await;
                continue;
            }
        };
        let status = resp.status();
        let headers = resp.headers().clone();
        if retry::should_retry(status, &headers, attempt, HN_MAX_RETRIES) {
            let delay = retry::retry_delay(&headers, attempt);
            eprintln!(
                "  retryable HTTP {status} on Hacker News search (attempt {attempt}/{HN_MAX_RETRIES}); retrying in {delay:?}"
            );
            sleep(delay).await;
            continue;
        }
        break (resp, status);
    };

    if !status.is_success() {
        anyhow::bail!("Hacker News HTTP {status}");
    }
    let v = json_response(resp).await?;
    Ok(parse_hn_hits(&v))
}

/// Pure transformation of an Algolia HN Search response body into
/// `ExternalMention`s — no HTTP, no I/O. Split out specifically so it's
/// unit-testable directly against representative and malformed payloads,
/// the same policy/mechanism split `retry.rs` uses. Before this was
/// extracted, the only "test" for this logic built its own `json!` value
/// and asserted against that value directly — it never actually called
/// the parsing code, so it could not have caught a real regression here.
fn parse_hn_hits(v: &Value) -> Vec<ExternalMention> {
    let mut out = vec![];
    if let Some(hits) = v.get("hits").and_then(|h| h.as_array()) {
        for hit in hits {
            out.push(ExternalMention {
                source: "hacker_news".into(),
                title: hit
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("(untitled)")
                    .to_string(),
                url: hit
                    .get("objectID")
                    .and_then(|id| id.as_str())
                    .map(|id| format!("https://news.ycombinator.com/item?id={id}"))
                    .unwrap_or_default(),
                score: hit.get("points").and_then(|s| s.as_i64()),
                num_comments: hit.get("num_comments").and_then(|s| s.as_i64()),
                created_at: hit
                    .get("created_at")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }
    out
}

async fn json_response(resp: reqwest::Response) -> Result<Value> {
    let text = resp.text().await.context("reading response body")?;
    serde_json::from_str(&text).context("parsing response as JSON")
}

#[cfg(test)]
mod tests {
    use super::parse_hn_hits;

    #[test]
    fn parses_a_representative_algolia_hit_into_an_external_mention() {
        let sample = serde_json::json!({
            "hits": [
                {
                    "title": "Show HN: ghlinks",
                    "objectID": "12345",
                    "points": 42,
                    "num_comments": 7,
                    "created_at": "2026-01-01T00:00:00.000Z"
                }
            ]
        });
        let mentions = parse_hn_hits(&sample);
        assert_eq!(mentions.len(), 1);
        let m = &mentions[0];
        assert_eq!(m.source, "hacker_news");
        assert_eq!(m.title, "Show HN: ghlinks");
        assert_eq!(m.url, "https://news.ycombinator.com/item?id=12345");
        assert_eq!(m.score, Some(42));
        assert_eq!(m.num_comments, Some(7));
        assert_eq!(m.created_at.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    }

    #[test]
    fn multiple_hits_are_all_parsed_in_order() {
        let sample = serde_json::json!({
            "hits": [
                {"title": "First", "objectID": "1", "points": 1, "num_comments": 0},
                {"title": "Second", "objectID": "2", "points": 2, "num_comments": 1}
            ]
        });
        let mentions = parse_hn_hits(&sample);
        assert_eq!(mentions.len(), 2);
        assert_eq!(mentions[0].title, "First");
        assert_eq!(mentions[1].title, "Second");
    }

    #[test]
    fn missing_title_falls_back_to_untitled_rather_than_erroring() {
        let sample = serde_json::json!({
            "hits": [ {"objectID": "1", "points": 1, "num_comments": 0} ]
        });
        let mentions = parse_hn_hits(&sample);
        assert_eq!(mentions[0].title, "(untitled)");
    }

    #[test]
    fn missing_object_id_produces_an_empty_url_rather_than_erroring() {
        let sample = serde_json::json!({
            "hits": [ {"title": "No id", "points": 1, "num_comments": 0} ]
        });
        let mentions = parse_hn_hits(&sample);
        assert_eq!(mentions[0].url, "");
    }

    #[test]
    fn missing_score_and_comment_fields_become_none_not_zero() {
        // Distinguishing "the API didn't report this" (None) from a
        // genuine zero matters for the same reason it matters at the
        // discovery-status level: a caller must not read an absent value
        // as a confirmed zero.
        let sample = serde_json::json!({
            "hits": [ {"title": "Sparse", "objectID": "1"} ]
        });
        let mentions = parse_hn_hits(&sample);
        assert_eq!(mentions[0].score, None);
        assert_eq!(mentions[0].num_comments, None);
        assert_eq!(mentions[0].created_at, None);
    }

    #[test]
    fn absent_hits_key_returns_an_empty_vec_not_an_error() {
        let sample = serde_json::json!({});
        assert!(parse_hn_hits(&sample).is_empty());
    }

    #[test]
    fn hits_present_but_not_an_array_returns_an_empty_vec() {
        let sample = serde_json::json!({"hits": "unexpected string shape"});
        assert!(parse_hn_hits(&sample).is_empty());
    }

    #[test]
    fn empty_hits_array_returns_an_empty_vec() {
        let sample = serde_json::json!({"hits": []});
        assert!(parse_hn_hits(&sample).is_empty());
    }
}

/// HTTP-boundary integration tests for `hacker_news()`, mirroring
/// `github.rs::http_boundary_tests` — the parser tests above prove
/// `parse_hn_hits` handles a given JSON shape correctly; these prove the
/// real `reqwest` call, against real HTTP responses via `base_url`, does
/// the right thing at success/empty/malformed/failure boundaries. Adding
/// these was the direct payoff of giving `hacker_news()` a `base_url`
/// parameter: before that, this endpoint could not be pointed at
/// anything but the live Algolia API.
#[cfg(test)]
mod http_boundary_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn a_hit_goes_through_real_http_to_an_external_mention() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "hits": [{
                    "title": "Show HN: ghlinks",
                    "objectID": "12345",
                    "points": 42,
                    "num_comments": 7,
                    "created_at": "2026-01-01T00:00:00.000Z"
                }]
            })))
            .mount(&server)
            .await;

        let client = Client::new();
        let mentions = hacker_news(&client, "https://github.com/owner/repo", &server.uri())
            .await
            .unwrap();

        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].title, "Show HN: ghlinks");
    }

    // -- zero hits is a valid, successful answer — not an error --
    #[tokio::test]
    async fn zero_hits_is_ok_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"hits": []})))
            .mount(&server)
            .await;

        let client = Client::new();
        let mentions = hacker_news(&client, "https://github.com/owner/repo", &server.uri())
            .await
            .unwrap();
        assert!(mentions.is_empty());
    }

    #[tokio::test]
    async fn malformed_response_body_is_reported_as_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let client = Client::new();
        let result = hacker_news(&client, "https://github.com/owner/repo", &server.uri()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn persistent_server_errors_fail_after_exhausting_the_hn_retry_budget() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = Client::new();
        let result = hacker_news(&client, "https://github.com/owner/repo", &server.uri()).await;

        assert!(result.is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            HN_MAX_RETRIES as usize,
            "expected exactly HN_MAX_RETRIES attempts, no more, no fewer"
        );
    }

    #[tokio::test]
    async fn plain_404_is_reported_without_retrying() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/search"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = Client::new();
        let result = hacker_news(&client, "https://github.com/owner/repo", &server.uri()).await;

        assert!(result.is_err());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "a plain 404 must not be retried");
    }
}
