mod classify;
mod discovery;
mod github;
mod model;

use anyhow::{Context, Result};
use clap::Parser;
use classify::{classify, LinkKind};
use futures::stream::{self, StreamExt};
use github::GitHub;
use model::*;
use reqwest::Client;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deterministic collector for a list of GitHub-hosted links. Reads one URL
/// per line, classifies each, pulls structured facts from GitHub's API, and
/// (optionally) checks free discovery APIs for external mentions. Writes a
/// single JSON array — no synthesis, summarization, or sentiment judgment
/// happens here; that's left for a human or an LLM working from this output.
#[derive(Parser, Debug)]
#[command(name = "ghlinks", version, about)]
struct Args {
    /// Path to a text file with one URL per line (blank lines and lines
    /// starting with # are ignored)
    #[arg(short, long)]
    input: PathBuf,

    /// Where to write the JSON report
    #[arg(short, long, default_value = "ghlinks-report.json")]
    output: PathBuf,

    /// GitHub personal access token. No scopes are required for public
    /// data, but a token raises your rate limit from 60/hr to 5000/hr and
    /// is required for the GraphQL endpoint entirely.
    #[arg(long, env = "GITHUB_TOKEN")]
    token: Option<String>,

    /// Max links processed concurrently
    #[arg(long, default_value_t = 3)]
    concurrency: usize,

    /// Delay between successive GitHub API calls within a single link's
    /// processing, in milliseconds. Keeps bursts polite; raise this if you
    /// see secondary rate-limit errors.
    #[arg(long, default_value_t = 250)]
    delay_ms: u64,

    /// Skip Hacker News external-mention lookups entirely
    #[arg(long, default_value_t = false)]
    skip_external: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.concurrency == 0 {
        anyhow::bail!("--concurrency must be at least 1");
    }

    if args.token.is_none() {
        eprintln!(
            "Warning: no GitHub token supplied (--token or $GITHUB_TOKEN).\n\
             The GraphQL endpoint used for most fields REQUIRES authentication.\n\
             Create a token with no scopes selected at https://github.com/settings/tokens\n\
             (public data only needs an unscoped 'classic' token, or a fine-grained\n\
             token with no repository access granted)."
        );
    }

    let raw =
        fs::read_to_string(&args.input).with_context(|| format!("reading {:?}", args.input))?;
    let urls: Vec<String> = raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    eprintln!("Loaded {} links from {:?}", urls.len(), args.input);

    let client = Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("building HTTP client")?;
    let gh = Arc::new(GitHub::new(client.clone(), args.token.clone()));
    let client = Arc::new(client);
    let delay = Duration::from_millis(args.delay_ms);
    let skip_external = args.skip_external;
    let total = urls.len();

    let results: Vec<LinkRecord> = stream::iter(urls.into_iter().enumerate())
        .map(|(i, url)| {
            let gh = gh.clone();
            let client = client.clone();
            async move {
                let record = process_link(&url, &gh, &client, delay, skip_external).await;
                eprintln!("[{}/{}] done: {}", i + 1, total, url);
                record
            }
        })
        .buffer_unordered(args.concurrency)
        .collect()
        .await;

    let json = serde_json::to_string_pretty(&results)?;
    fs::write(&args.output, json).with_context(|| format!("writing {:?}", args.output))?;

    let ok = results.iter().filter(|r| r.fetch_errors.is_empty()).count();
    eprintln!(
        "Wrote {} records to {:?} ({} with no errors, {} with at least one error)",
        results.len(),
        args.output,
        ok,
        results.len() - ok
    );
    Ok(())
}

async fn process_link(
    url: &str,
    gh: &GitHub,
    client: &Client,
    delay: Duration,
    skip_external: bool,
) -> LinkRecord {
    let mut errors = vec![];
    let kind = classify(url);
    let (kind_name, owner, repo, file_path) = describe_kind(&kind);

    let mut repo_data = None;
    let mut gist_data = None;
    let mut pages_checked = vec![];
    let mut pages_resolved = None;

    match &kind {
        LinkKind::RepoRoot { owner, repo } | LinkKind::RepoFile { owner, repo, .. } => {
            repo_data = collect_repo_data(owner, repo, gh, delay, &mut errors).await;
        }
        LinkKind::Gist { gist_id, .. } => match gh.gist(gist_id).await {
            Ok(v) => gist_data = Some(parse_gist_data(&v)),
            Err(e) => errors.push(format!("gist: {e}")),
        },
        LinkKind::PagesSite { candidates, .. } => {
            for (cand_owner, cand_repo) in candidates {
                pages_checked.push(format!("{cand_owner}/{cand_repo}"));
                match gh.repo_exists(cand_owner, cand_repo).await {
                    Ok(true) => {
                        pages_resolved = Some(format!("{cand_owner}/{cand_repo}"));
                        repo_data =
                            collect_repo_data(cand_owner, cand_repo, gh, delay, &mut errors).await;
                        break;
                    }
                    Ok(false) => {}
                    Err(e) => errors.push(format!("repo_exists({cand_owner}/{cand_repo}): {e}")),
                }
                sleep(delay).await;
            }
            if pages_resolved.is_none() {
                errors.push(
                    "no candidate repo resolved automatically for this Pages site; \
                     manual lookup needed"
                        .into(),
                );
            }
        }
        LinkKind::Unknown => {
            errors.push("could not classify URL (unrecognized host/path shape)".into())
        }
        LinkKind::UnsupportedGithubUrl => errors.push(
            "recognized GitHub subpage is not a repository root or file link; it was not collected"
                .into(),
        ),
    }

    sleep(delay).await;

    let mut external = vec![];
    if !skip_external {
        match discovery::hacker_news(client, url).await {
            Ok(mut v) => external.append(&mut v),
            Err(e) => errors.push(format!("hacker_news: {e}")),
        }
    }

    LinkRecord {
        input_url: url.to_string(),
        link_kind: kind_name,
        owner,
        repo,
        file_path,
        repo_data,
        gist_data,
        pages_candidates_checked: pages_checked,
        pages_resolved_repo: pages_resolved,
        external_mentions: external,
        external_discovery: ExternalDiscovery {
            skipped: skip_external,
            sources: if skip_external { vec![] } else { vec!["hacker_news"] },
            coverage: "Discovery signals only: HN stories linking to the exact URL. Comments and other sites are not searched by ghlinks — Reddit mentions are discovered during the downstream LLM synthesis pass instead (see ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md).",
            hacker_news_query: "exact URL; stories only",
        },
        fetch_errors: errors,
        fetched_at: chrono::Utc::now().to_rfc3339(),
        collector_version: env!("CARGO_PKG_VERSION"),
    }
}

async fn collect_repo_data(
    owner: &str,
    repo: &str,
    gh: &GitHub,
    delay: Duration,
    errors: &mut Vec<String>,
) -> Option<RepoData> {
    let value = match gh.graphql_repo(owner, repo).await {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!("graphql_repo: {error}"));
            return None;
        }
    };
    let mut data = parse_repo_data(&value);
    sleep(delay).await;
    match gh.languages(owner, repo).await {
        Ok(languages) => data.languages_bytes = languages,
        Err(error) => errors.push(format!("languages: {error}")),
    }
    sleep(delay).await;
    match gh.contributors_count(owner, repo).await {
        Ok(count) => data.github_contributors_count = count,
        Err(error) => errors.push(format!("contributors_count: {error}")),
    }
    sleep(delay).await;
    match gh.releases(owner, repo).await {
        Ok(releases) => apply_releases(&mut data, &releases),
        Err(error) => errors.push(format!("releases: {error}")),
    }
    Some(data)
}

fn describe_kind(kind: &LinkKind) -> (String, Option<String>, Option<String>, Option<String>) {
    match kind {
        LinkKind::RepoRoot { owner, repo } => (
            "repo_root".into(),
            Some(owner.clone()),
            Some(repo.clone()),
            None,
        ),
        LinkKind::RepoFile {
            owner, repo, path, ..
        } => (
            "repo_file".into(),
            Some(owner.clone()),
            Some(repo.clone()),
            Some(path.clone()),
        ),
        LinkKind::Gist { owner, gist_id } => (
            "gist".into(),
            Some(owner.clone()),
            Some(gist_id.clone()),
            None,
        ),
        LinkKind::PagesSite { owner, path, .. } => (
            "pages_site".into(),
            Some(owner.clone()),
            None,
            Some(path.clone()),
        ),
        LinkKind::UnsupportedGithubUrl => ("unsupported_github_url".into(), None, None, None),
        LinkKind::Unknown => ("unknown".into(), None, None, None),
    }
}

fn parse_repo_data(v: &Value) -> RepoData {
    let repo = &v["data"]["repository"];

    RepoData {
        description: repo["description"].as_str().map(|s| s.to_string()),
        homepage: repo["homepageUrl"].as_str().map(|s| s.to_string()),
        license_key: repo["licenseInfo"]["key"].as_str().map(|s| s.to_string()),
        license_name: repo["licenseInfo"]["name"].as_str().map(|s| s.to_string()),
        stargazers_count: repo["stargazerCount"].as_i64(),
        forks_count: repo["forkCount"].as_i64(),
        watchers_count: repo["watchers"]["totalCount"].as_i64(),
        open_issues_count: repo["openIssues"]["totalCount"].as_i64(),
        closed_issues_count: repo["closedIssues"]["totalCount"].as_i64(),
        primary_language: repo["primaryLanguage"]["name"]
            .as_str()
            .map(|s| s.to_string()),
        languages_bytes: Default::default(),
        created_at: repo["createdAt"].as_str().map(|s| s.to_string()),
        pushed_at: repo["pushedAt"].as_str().map(|s| s.to_string()),
        default_branch: repo["defaultBranchRef"]["name"]
            .as_str()
            .map(|s| s.to_string()),
        commit_count_default_branch: repo["defaultBranchRef"]["target"]["history"]["totalCount"]
            .as_i64(),
        topics: repo["repositoryTopics"]["nodes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n["topic"]["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        github_contributors_count: None,
        github_contributors_count_semantics:
            "GitHub contributors endpoint entries; anon=true; not unique humans",
        releases_total_count: repo["releases"]["totalCount"].as_i64(),
        releases_last_12_months: None,
        latest_release_tag: None,
        latest_release_published_at: None,
        recent_releases: vec![],
    }
}

fn apply_releases(repo: &mut RepoData, releases: &[Value]) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(365);
    repo.releases_last_12_months = Some(
        releases
            .iter()
            .filter(|release| {
                release["publishedAt"]
                    .as_str()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|published| published.with_timezone(&chrono::Utc) >= cutoff)
            })
            .count() as i64,
    );
    repo.recent_releases = releases
        .iter()
        .take(100)
        .map(|release| ReleaseEntry {
            tag_name: release["tagName"].as_str().map(str::to_owned),
            name: release["name"].as_str().map(str::to_owned),
            published_at: release["publishedAt"].as_str().map(str::to_owned),
        })
        .collect();
    repo.latest_release_tag = repo
        .recent_releases
        .first()
        .and_then(|release| release.tag_name.clone());
    repo.latest_release_published_at = repo
        .recent_releases
        .first()
        .and_then(|release| release.published_at.clone());
}

fn parse_gist_data(v: &Value) -> GistData {
    let files = v["files"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let revision_count = v["history"].as_array().map(|a| a.len() as i64);
    GistData {
        description: v["description"].as_str().map(|s| s.to_string()),
        owner_login: v["owner"]["login"].as_str().map(|s| s.to_string()),
        created_at: v["created_at"].as_str().map(|s| s.to_string()),
        updated_at: v["updated_at"].as_str().map(|s| s.to_string()),
        comments: v["comments"].as_i64(),
        revision_count,
        files,
        note: Some(
            "GitHub's REST API does not expose gist star/fork counts; read those from \
             the gist's HTML page if needed."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_releases, parse_repo_data};
    use serde_json::json;

    #[test]
    fn releases_are_counted_from_all_supplied_pages_and_sampled_to_100() {
        let mut repo =
            parse_repo_data(&json!({"data":{"repository":{"releases":{"totalCount":101}}}}));
        let releases: Vec<_> = (0..101).map(|i| json!({"tagName":format!("v{i}"), "name":null, "publishedAt":"2026-08-01T00:00:00Z"})).collect();
        apply_releases(&mut repo, &releases);
        assert_eq!(repo.releases_last_12_months, Some(101));
        assert_eq!(repo.recent_releases.len(), 100);
    }

    #[test]
    fn invalid_or_old_release_dates_are_not_counted() {
        let mut repo =
            parse_repo_data(&json!({"data":{"repository":{"releases":{"totalCount":2}}}}));
        apply_releases(
            &mut repo,
            &[
                json!({"tagName":"old", "publishedAt":"2000-01-01T00:00:00Z"}),
                json!({"tagName":"bad", "publishedAt":"not-a-date"}),
            ],
        );
        assert_eq!(repo.releases_last_12_months, Some(0));
    }
}
