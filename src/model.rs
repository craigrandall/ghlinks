use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Default, Debug)]
pub struct ReleaseEntry {
    pub tag_name: Option<String>,
    pub name: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Serialize, Default, Debug)]
pub struct RepoData {
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub license_key: Option<String>,
    pub license_name: Option<String>,
    pub stargazers_count: Option<i64>,
    pub forks_count: Option<i64>,
    pub watchers_count: Option<i64>,
    pub open_issues_count: Option<i64>,
    pub closed_issues_count: Option<i64>,
    pub primary_language: Option<String>,
    /// language name -> bytes of code (GitHub's own linguist breakdown)
    pub languages_bytes: BTreeMap<String, i64>,
    pub created_at: Option<String>,
    pub pushed_at: Option<String>,
    pub default_branch: Option<String>,
    pub commit_count_default_branch: Option<i64>,
    pub topics: Vec<String>,
    /// Count of entries in GitHub's contributors endpoint, including
    /// anonymous entries when GitHub returns them. It is not a count of
    /// unique human contributors.
    pub github_contributors_count: Option<i64>,
    pub github_contributors_count_semantics: &'static str,
    pub releases_total_count: Option<i64>,
    pub releases_last_12_months: Option<i64>,
    pub latest_release_tag: Option<String>,
    pub latest_release_published_at: Option<String>,
    /// Up to the 100 most recent releases (GraphQL page cap used here).
    pub recent_releases: Vec<ReleaseEntry>,
}

#[derive(Serialize, Default, Debug)]
pub struct GistData {
    pub description: Option<String>,
    pub owner_login: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments: Option<i64>,
    /// Length of the gist's revision history array.
    pub revision_count: Option<i64>,
    pub files: Vec<String>,
    pub note: Option<String>,
}

#[derive(Serialize, Default, Debug)]
pub struct ExternalMention {
    pub source: String, // "hacker_news" (see ADR: reddit-mention-discovery-moves-to-synthesis-pass)
    pub title: String,
    pub url: String,
    pub score: Option<i64>,
    pub num_comments: Option<i64>,
    pub created_at: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct ExternalDiscovery {
    pub skipped: bool,
    pub sources: Vec<&'static str>,
    pub coverage: &'static str,
    pub hacker_news_query: &'static str,
}

#[derive(Serialize, Debug)]
pub struct LinkRecord {
    pub input_url: String,
    pub link_kind: String,
    pub owner: Option<String>,
    pub repo: Option<String>,
    pub file_path: Option<String>,
    pub repo_data: Option<RepoData>,
    pub gist_data: Option<GistData>,
    pub pages_candidates_checked: Vec<String>,
    pub pages_resolved_repo: Option<String>,
    pub external_mentions: Vec<ExternalMention>,
    pub external_discovery: ExternalDiscovery,
    pub fetch_errors: Vec<String>,
    pub fetched_at: String,
    pub collector_version: &'static str,
}
