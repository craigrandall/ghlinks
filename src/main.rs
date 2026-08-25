mod classify;
mod discovery;
mod github;
mod model;
mod retry;

use anyhow::{Context, Result};
use clap::Parser;
use classify::{canonicalize, classify, LinkKind};
use futures::stream::{self, StreamExt};
use github::GitHub;
use model::*;
use reqwest::Client;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

/// Deterministic collector for a list of GitHub-hosted links. Reads one URL
/// per line, classifies each, pulls structured facts from GitHub's API, and
/// (optionally) checks free discovery APIs for external mentions. Writes a
/// single JSON object — no synthesis, summarization, or sentiment judgment
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

    /// GitHub personal access token. Hidden from --help on purpose: prefer
    /// setting $GITHUB_TOKEN, or let run.ps1 prompt you for it securely.
    /// A token passed as a literal CLI argument can land in shell history
    /// and process listings (e.g. `ps aux` on a shared machine). This flag
    /// still works — for scripts that already depend on it — it's just not
    /// advertised as the preferred path.
    #[arg(long, env = "GITHUB_TOKEN", hide = true)]
    token: Option<String>,

    /// Max links processed concurrently
    #[arg(long, default_value_t = 3)]
    concurrency: usize,

    /// Delay between successive GitHub API calls within a single link's
    /// processing, in milliseconds. Keeps bursts polite; raise this if you
    /// see secondary rate-limit errors.
    #[arg(long, default_value_t = 250)]
    delay_ms: u64,

    /// Per-HTTP-request timeout, in seconds
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,

    /// Max attempts per GitHub API call (including the first) before
    /// giving up on it — applies to transient network errors, HTTP 5xx,
    /// 429, and 403 responses GitHub marks as rate-limit-related. Hacker
    /// News discovery uses its own fixed retry budget, independent of
    /// this flag.
    #[arg(long, default_value_t = 3)]
    max_retries: u32,

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
    if args.max_retries == 0 {
        anyhow::bail!("--max-retries must be at least 1");
    }
    if args.timeout_secs == 0 {
        anyhow::bail!("--timeout-secs must be at least 1");
    }

    if args.token.is_none() {
        eprintln!(
            "Warning: no GitHub token supplied ($GITHUB_TOKEN is the preferred way to \
             set one; run.ps1 will prompt you securely if you don't have one set).\n\
             The GraphQL endpoint used for most fields REQUIRES authentication.\n\
             Create a token with no scopes selected at https://github.com/settings/tokens\n\
             (public data only needs an unscoped 'classic' token, or a fine-grained\n\
             token with no repository access granted)."
        );
    }

    let started_at = chrono::Utc::now();

    let raw =
        fs::read_to_string(&args.input).with_context(|| format!("reading {:?}", args.input))?;
    let urls: Vec<String> = raw
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    eprintln!("Loaded {} links from {:?}", urls.len(), args.input);

    let client = Client::builder()
        .timeout(Duration::from_secs(args.timeout_secs))
        .build()
        .context("building HTTP client")?;
    let gh = Arc::new(GitHub::new(client.clone(), args.token.clone(), args.max_retries));
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

    let finished_at = chrono::Utc::now();

    let mut link_kind_counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &results {
        *link_kind_counts.entry(r.link_kind.clone()).or_insert(0) += 1;
    }
    let records_with_errors = results.iter().filter(|r| !r.fetch_errors.is_empty()).count();
    let total_records = results.len();

    let report = Report {
        schema_version: model::SCHEMA_VERSION,
        run_summary: RunSummary {
            ghlinks_version: env!("CARGO_PKG_VERSION"),
            github_api_version: github::GITHUB_API_VERSION,
            hacker_news_api: "hn.algolia.com/api/v1/search (Algolia-backed HN Search API)",
            reddit_note: "Reddit is not queried by ghlinks; see ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md",
            started_at: started_at.to_rfc3339(),
            finished_at: finished_at.to_rfc3339(),
            input_file: args.input.display().to_string(),
            total_urls: total,
            link_kind_counts,
            records_with_errors,
            concurrency: args.concurrency,
            delay_ms: args.delay_ms,
            timeout_secs: args.timeout_secs,
            max_retries: args.max_retries,
            skip_external,
        },
        records: results,
    };

    let json = serde_json::to_string_pretty(&report)?;
    fs::write(&args.output, json).with_context(|| format!("writing {:?}", args.output))?;

    eprintln!(
        "Wrote {} records to {:?} ({} with no errors, {} with at least one error)",
        total_records,
        args.output,
        total_records - records_with_errors,
        records_with_errors
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
    let canonical_url = canonicalize(url);

    let mut repo_data = None;
    let mut gist_data = None;
    let mut pages_checked = vec![];
    let mut pages_resolved = None;

    match &kind {
        LinkKind::RepoRoot { owner, repo } | LinkKind::RepoFile { owner, repo, .. } => {
            repo_data = collect_repo_data(owner, repo, gh, delay, &mut errors).await;
        }
        LinkKind::Gist { gist_id, .. } => match gh.gist(gist_id).await {
            Ok(g) => gist_data = Some(build_gist_data(&g)),
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
        LinkKind::UserOrOrgProfile { .. } => errors.push(
            "recognized as a GitHub user/organization profile page, not a repository; \
             it was not collected"
                .into(),
        ),
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
    let mut hn_status: &'static str = "skipped";
    if !skip_external {
        match discovery::hacker_news(client, url).await {
            Ok(mut v) => {
                hn_status = "ok";
                external.append(&mut v);
            }
            Err(e) => {
                hn_status = "error";
                errors.push(format!("hacker_news: {e}"));
            }
        }
    }
    let hn_count = external.len();

    LinkRecord {
        input_url: url.to_string(),
        canonical_url,
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
            hacker_news_status: hn_status,
            hacker_news_mention_count: hn_count,
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
    let node = match gh.graphql_repo(owner, repo).await {
        Ok(node) => node,
        Err(error) => {
            errors.push(format!("graphql_repo: {error}"));
            return None;
        }
    };
    let mut data = build_repo_data(&node);
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
        LinkKind::UserOrOrgProfile { login } => (
            "user_or_org_profile".into(),
            Some(login.clone()),
            None,
            None,
        ),
        LinkKind::UnsupportedGithubUrl => ("unsupported_github_url".into(), None, None, None),
        LinkKind::Unknown => ("unknown".into(), None, None, None),
    }
}

fn build_repo_data(node: &github::RepositoryNode) -> RepoData {
    RepoData {
        description: node.description.clone(),
        homepage: node.homepage_url.clone(),
        license_key: node.license_info.as_ref().and_then(|l| l.key.clone()),
        license_name: node.license_info.as_ref().and_then(|l| l.name.clone()),
        stargazers_count: node.stargazer_count,
        forks_count: node.fork_count,
        watchers_count: node.watchers.as_ref().and_then(|w| w.total_count),
        open_issues_count: node.open_issues.as_ref().and_then(|w| w.total_count),
        closed_issues_count: node.closed_issues.as_ref().and_then(|w| w.total_count),
        primary_language: node.primary_language.as_ref().and_then(|l| l.name.clone()),
        languages_bytes: Default::default(),
        created_at: node.created_at.clone(),
        pushed_at: node.pushed_at.clone(),
        default_branch: node.default_branch_ref.as_ref().and_then(|b| b.name.clone()),
        commit_count_default_branch: node
            .default_branch_ref
            .as_ref()
            .and_then(|b| b.target.as_ref())
            .and_then(|t| t.history.as_ref())
            .and_then(|h| h.total_count),
        topics: node
            .repository_topics
            .as_ref()
            .map(|t| {
                t.nodes
                    .iter()
                    .filter_map(|n| n.topic.as_ref().and_then(|topic| topic.name.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        github_contributors_count: None,
        github_contributors_count_semantics:
            "GitHub contributors endpoint entries; anon=true; not unique humans",
        releases_total_count: node.releases.as_ref().and_then(|r| r.total_count),
        releases_last_12_months: None,
        latest_release_tag: None,
        latest_release_published_at: None,
        recent_releases: vec![],
    }
}

fn apply_releases(repo: &mut RepoData, releases: &[github::ReleaseNode]) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(365);
    repo.releases_last_12_months = Some(
        releases
            .iter()
            .filter(|release| {
                release
                    .published_at
                    .as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|published| published.with_timezone(&chrono::Utc) >= cutoff)
            })
            .count() as i64,
    );
    repo.recent_releases = releases
        .iter()
        .take(100)
        .map(|release| ReleaseEntry {
            tag_name: release.tag_name.clone(),
            name: release.name.clone(),
            published_at: release.published_at.clone(),
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

fn build_gist_data(g: &github::GistResponse) -> GistData {
    GistData {
        description: g.description.clone(),
        owner_login: g.owner.as_ref().and_then(|o| o.login.clone()),
        created_at: g.created_at.clone(),
        updated_at: g.updated_at.clone(),
        comments: g.comments,
        revision_count: Some(g.history.len() as i64),
        files: g.files.keys().cloned().collect(),
        note: Some(
            "GitHub's REST API does not expose gist star/fork counts; read those from \
             the gist's HTML page if needed."
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_releases, build_gist_data, build_repo_data, describe_kind};
    use crate::classify::LinkKind;
    use crate::github::{CountWrapper, GistOwner, GistResponse, ReleaseNode, RepositoryNode};
    use std::collections::BTreeMap;

    #[test]
    fn releases_are_counted_from_all_supplied_pages_and_sampled_to_100() {
        let node = RepositoryNode {
            releases: Some(CountWrapper {
                total_count: Some(101),
            }),
            ..Default::default()
        };
        let mut repo = build_repo_data(&node);
        let releases: Vec<ReleaseNode> = (0..101)
            .map(|i| ReleaseNode {
                tag_name: Some(format!("v{i}")),
                name: None,
                published_at: Some("2026-08-01T00:00:00Z".to_string()),
            })
            .collect();
        apply_releases(&mut repo, &releases);
        assert_eq!(repo.releases_last_12_months, Some(101));
        assert_eq!(repo.recent_releases.len(), 100);
    }

    #[test]
    fn invalid_or_old_release_dates_are_not_counted() {
        let node = RepositoryNode {
            releases: Some(CountWrapper { total_count: Some(2) }),
            ..Default::default()
        };
        let mut repo = build_repo_data(&node);
        apply_releases(
            &mut repo,
            &[
                ReleaseNode {
                    tag_name: Some("old".into()),
                    name: None,
                    published_at: Some("2000-01-01T00:00:00Z".into()),
                },
                ReleaseNode {
                    tag_name: Some("bad".into()),
                    name: None,
                    published_at: Some("not-a-date".into()),
                },
            ],
        );
        assert_eq!(repo.releases_last_12_months, Some(0));
    }

    #[test]
    fn latest_release_reflects_the_first_recent_release_entry() {
        let node = RepositoryNode::default();
        let mut repo = build_repo_data(&node);
        apply_releases(
            &mut repo,
            &[ReleaseNode {
                tag_name: Some("v3".into()),
                name: Some("Three".into()),
                published_at: Some("2026-06-01T00:00:00Z".into()),
            }],
        );
        assert_eq!(repo.latest_release_tag.as_deref(), Some("v3"));
        assert_eq!(
            repo.latest_release_published_at.as_deref(),
            Some("2026-06-01T00:00:00Z")
        );
    }

    #[test]
    fn build_gist_data_reads_owner_and_file_names_and_leaves_a_star_count_note() {
        let mut files = BTreeMap::new();
        files.insert("a.py".to_string(), serde_json::json!({"filename": "a.py"}));
        files.insert("b.py".to_string(), serde_json::json!({"filename": "b.py"}));
        let gist = GistResponse {
            description: Some("desc".into()),
            owner: Some(GistOwner {
                login: Some("octocat".into()),
            }),
            comments: Some(1),
            history: vec![serde_json::json!({}), serde_json::json!({})],
            files,
            ..Default::default()
        };
        let data = build_gist_data(&gist);
        assert_eq!(data.owner_login.as_deref(), Some("octocat"));
        assert_eq!(data.revision_count, Some(2));
        assert_eq!(data.files.len(), 2);
        assert!(data.note.unwrap().contains("star/fork counts"));
    }

    #[test]
    fn user_or_org_profile_is_described_but_not_treated_as_a_repo() {
        let (kind_name, owner, repo, path) =
            describe_kind(&LinkKind::UserOrOrgProfile { login: "octocat".into() });
        assert_eq!(kind_name, "user_or_org_profile");
        assert_eq!(owner.as_deref(), Some("octocat"));
        assert!(repo.is_none());
        assert!(path.is_none());
    }
}