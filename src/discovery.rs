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

/// Hacker News stories linking to the exact URL, via Algolia's HN Search
/// API (`restrictSearchableAttributes=url`). Only stories are searched,
/// not comments — comment-level mentions of a URL are a different, much
/// noisier signal this tool deliberately doesn't chase.
pub async fn hacker_news(client: &Client, target_url: &str) -> Result<Vec<ExternalMention>> {
    let endpoint = format!(
        "https://hn.algolia.com/api/v1/search?query={}&restrictSearchableAttributes=url&tags=story",
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
    Ok(out)
}

async fn json_response(resp: reqwest::Response) -> Result<Value> {
    let text = resp.text().await.context("reading response body")?;
    serde_json::from_str(&text).context("parsing response as JSON")
}

#[cfg(test)]
mod tests {

    #[test]
    fn parses_a_representative_algolia_hit() {
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
        let hits = sample.get("hits").and_then(|h| h.as_array()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["points"], 42);
    }
}
