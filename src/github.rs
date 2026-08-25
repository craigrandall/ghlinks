use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::BTreeMap;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct GitHub {
    client: Client,
    token: Option<String>,
}

impl GitHub {
    pub fn new(client: Client, token: Option<String>) -> Self {
        Self { client, token }
    }

    fn auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req = req
            .header(
                "User-Agent",
                concat!("ghlinks-collector/", env!("CARGO_PKG_VERSION")),
            )
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = &self.token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        req
    }

    /// One GraphQL round-trip pulls description, license, star/fork/watch
    /// counts, open+closed issue totals, primary language, timestamps,
    /// default branch commit count, topics, and release total count.
    pub async fn graphql_repo(&self, owner: &str, name: &str) -> Result<Value> {
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
        let req = self
            .client
            .post("https://api.github.com/graphql")
            .json(&body);
        let resp = self
            .auth(req)
            .send()
            .await
            .context("graphql request failed")?;
        let status = resp.status();
        let text = response_text(resp)
            .await
            .context("reading graphql response body")?;
        if !status.is_success() {
            anyhow::bail!("GraphQL HTTP {status}: {text}");
        }
        let v: Value = serde_json::from_str(&text).context("parsing graphql json")?;
        if let Some(errors) = v.get("errors") {
            anyhow::bail!("GraphQL returned errors: {errors}");
        }
        if v["data"]["repository"].is_null() {
            anyhow::bail!("repository {owner}/{name} not found or inaccessible");
        }
        Ok(v)
    }

    /// The REST languages endpoint has no GraphQL connection page cap.
    pub async fn languages(&self, owner: &str, repo: &str) -> Result<BTreeMap<String, i64>> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}/languages");
        let resp = self.auth(self.client.get(url)).send().await?;
        let status = resp.status();
        let text = response_text(resp).await?;
        if !status.is_success() {
            anyhow::bail!("languages HTTP {status}: {text}");
        }
        serde_json::from_str(&text).context("parsing languages json")
    }

    /// Pages every release connection so the 12-month rollup is not limited
    /// to an arbitrary first page. The returned list is newest-created first.
    pub async fn releases(&self, owner: &str, name: &str) -> Result<Vec<Value>> {
        let query = r#"query($owner:String!, $name:String!, $after:String) {
          repository(owner:$owner, name:$name) {
            releases(first:100, after:$after, orderBy:{field:CREATED_AT,direction:DESC}) {
              nodes { tagName name publishedAt }
              pageInfo { hasNextPage endCursor }
            }
          }
        }"#;
        let mut after: Option<String> = None;
        let mut releases = Vec::new();
        loop {
            let body =
                json!({"query": query, "variables": {"owner":owner,"name":name,"after":after}});
            let resp = self
                .auth(
                    self.client
                        .post("https://api.github.com/graphql")
                        .json(&body),
                )
                .send()
                .await?;
            let status = resp.status();
            let text = response_text(resp).await?;
            if !status.is_success() {
                anyhow::bail!("releases GraphQL HTTP {status}: {text}");
            }
            let value: Value = serde_json::from_str(&text)?;
            if let Some(errors) = value.get("errors") {
                anyhow::bail!("releases GraphQL returned errors: {errors}");
            }
            let connection = &value["data"]["repository"]["releases"];
            releases.extend(connection["nodes"].as_array().cloned().unwrap_or_default());
            if !connection["pageInfo"]["hasNextPage"]
                .as_bool()
                .unwrap_or(false)
            {
                break;
            }
            after = connection["pageInfo"]["endCursor"]
                .as_str()
                .map(str::to_owned);
            if after.is_none() {
                anyhow::bail!("releases page claims another page but has no cursor");
            }
        }
        Ok(releases)
    }

    /// Exact contributor count without paging through every contributor:
    /// ask for 1 result per page and read the page number of the "last"
    /// rel in the response's Link header. That number *is* the total count.
    pub async fn contributors_count(&self, owner: &str, repo: &str) -> Result<Option<i64>> {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/contributors?per_page=1&anon=true"
        );
        let req = self.client.get(&url);
        let resp = self.auth(req).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = response_text(resp).await?;
            anyhow::bail!("contributors HTTP {status}: {text}");
        }
        let link_header = resp
            .headers()
            .get("link")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let text = response_text(resp).await?;
        let body: Value = serde_json::from_str(&text).context("parsing contributors json")?;

        if let Some(link) = link_header {
            if let Some(last) = parse_last_page(&link) {
                return Ok(Some(last));
            }
        }
        // No Link header at all means everything fit on page 1 (0 or 1 contributor).
        Ok(body.as_array().map(|a| a.len() as i64))
    }

    pub async fn repo_exists(&self, owner: &str, repo: &str) -> Result<bool> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}");
        let req = self.client.get(&url);
        let resp = self.auth(req).send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !status.is_success() {
            anyhow::bail!("repository existence HTTP {status}");
        }
        Ok(true)
    }

    pub async fn gist(&self, gist_id: &str) -> Result<Value> {
        let url = format!("https://api.github.com/gists/{gist_id}");
        let req = self.client.get(&url);
        let resp = self.auth(req).send().await?;
        let status = resp.status();
        let text = response_text(resp).await?;
        if !status.is_success() {
            anyhow::bail!("Gist HTTP {status}: {text}");
        }
        Ok(serde_json::from_str(&text)?)
    }
}

async fn response_text(resp: reqwest::Response) -> Result<String> {
    if resp
        .content_length()
        .is_some_and(|length| length as usize > MAX_RESPONSE_BYTES)
    {
        anyhow::bail!("response exceeds {MAX_RESPONSE_BYTES} byte limit");
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        anyhow::bail!("response exceeds {MAX_RESPONSE_BYTES} byte limit");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_last_page(link_header: &str) -> Option<i64> {
    // Link: <https://api.github.com/...&page=42>; rel="last", <...>; rel="next"
    for part in link_header.split(',') {
        if part.contains("rel=\"last\"") {
            let start = part.find('<')? + 1;
            let end = part.find('>')?;
            let url_str = &part[start..end];
            let u = url::Url::parse(url_str).ok()?;
            for (k, v) in u.query_pairs() {
                if k == "page" {
                    return v.parse::<i64>().ok();
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_last_page;
    #[test]
    fn finds_last_page_among_pagination_links() {
        assert_eq!(parse_last_page("<https://api.github.com/x?page=2>; rel=\"next\", <https://api.github.com/x?page=42>; rel=\"last\""), Some(42));
    }
    #[test]
    fn ignores_malformed_links() {
        assert_eq!(parse_last_page("not a link"), None);
    }
}
