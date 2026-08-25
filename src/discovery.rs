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

use crate::model::ExternalMention;
use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// Hacker News, via the Algolia-backed HN Search API. Restricting the
/// search to the `url` attribute finds stories/comments that link to this
/// exact URL, rather than just mentioning similar text.
pub async fn hacker_news(client: &Client, target_url: &str) -> Result<Vec<ExternalMention>> {
    let endpoint = format!(
        "https://hn.algolia.com/api/v1/search?query={}&restrictSearchableAttributes=url&tags=story",
        urlencoding::encode(target_url)
    );
    let resp = client.get(&endpoint).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Hacker News HTTP {}", resp.status());
    }
    let v = json_response(resp).await?;
    let mut out = vec![];
    if let Some(hits) = v.get("hits").and_then(|h| h.as_array()) {
        for hit in hits {
            let title = hit
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("(untitled)")
                .to_string();
            let object_id = hit.get("objectID").and_then(|t| t.as_str()).unwrap_or("");
            out.push(ExternalMention {
                source: "hacker_news".into(),
                title,
                url: format!("https://news.ycombinator.com/item?id={object_id}"),
                score: hit.get("points").and_then(|p| p.as_i64()),
                num_comments: hit.get("num_comments").and_then(|p| p.as_i64()),
                created_at: hit
                    .get("created_at")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }
    Ok(out)
}

async fn json_response(resp: reqwest::Response) -> Result<Value> {
    if resp
        .content_length()
        .is_some_and(|size| size as usize > MAX_RESPONSE_BYTES)
    {
        anyhow::bail!("response exceeds {MAX_RESPONSE_BYTES} byte limit");
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        anyhow::bail!("response exceeds {MAX_RESPONSE_BYTES} byte limit");
    }
    Ok(serde_json::from_slice(&bytes)?)
}
