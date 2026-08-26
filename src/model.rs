//! The `report.json` schema: every type in this file derives `Serialize`
//! and is written out, directly or nested, as part of `Report`. This
//! module is intentionally schema-only — no HTTP calls, no business
//! logic, no field computed from another field at construction time.
//! Collection modules (`github.rs`, `discovery.rs`) build these structs
//! from API responses; `main.rs` assembles them into a `Report` and
//! serializes it. If a change here isn't purely "add/rename/remove a
//! field," it likely belongs in a collection module instead.
//!
//! Several fields carry their own semantics caveats as doc-comments
//! (e.g. `github_contributors_count_semantics`, the ok/error/skipped
//! distinction on `ExternalDiscovery`) rather than leaving a downstream
//! reader to infer meaning from a bare number — see the README's "Known
//! limitations" section for the reasoning behind each one.

use serde::Serialize;
use std::collections::BTreeMap;

/// Bumped whenever the shape of the JSON this tool writes changes in a way
/// that could break a consumer parsing it. 1 was the original bare-array
/// shape (ghlinks <=0.12). 2 is the wrapped
/// `{schema_version, run_summary, records}` shape introduced in 0.13 —
/// see ADRs/ for the rationale if one gets written for this change.
pub const SCHEMA_VERSION: u32 = 2;

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
    pub languages_bytes: BTreeMap<String, i64>,
    pub created_at: Option<String>,
    pub pushed_at: Option<String>,
    pub default_branch: Option<String>,
    pub commit_count_default_branch: Option<i64>,
    pub topics: Vec<String>,
    pub github_contributors_count: Option<i64>,
    pub github_contributors_count_semantics: &'static str,
    pub releases_total_count: Option<i64>,
    pub releases_last_12_months: Option<i64>,
    pub latest_release_tag: Option<String>,
    pub latest_release_published_at: Option<String>,
    pub recent_releases: Vec<ReleaseEntry>,
}

#[derive(Serialize, Default, Debug)]
pub struct GistData {
    pub description: Option<String>,
    pub owner_login: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub comments: Option<i64>,
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

/// Per-source discovery status, distinguishing "the search ran and found
/// nothing" from "the search failed" — these must never be conflated.
/// `hacker_news_status == "ok"` with `hacker_news_mention_count == 0` is a
/// genuine zero-results finding. `hacker_news_status == "error"` means the
/// call failed; the reason is in that record's `fetch_errors`, and an
/// empty `external_mentions` in that case means "unknown", not "zero".
#[derive(Serialize, Debug)]
pub struct ExternalDiscovery {
    pub skipped: bool,
    pub sources: Vec<&'static str>,
    pub coverage: &'static str,
    pub hacker_news_query: &'static str,
    pub hacker_news_status: &'static str, // "ok" | "error" | "skipped"
    pub hacker_news_mention_count: usize,
}

#[derive(Serialize, Debug)]
pub struct LinkRecord {
    pub input_url: String,
    /// Normalized form used for cross-run comparison: scheme+host
    /// lowercased, a trailing `.git` and trailing slash trimmed, query
    /// string and fragment dropped. `None` only when the URL could not be
    /// parsed at all. See `classify::canonicalize`.
    pub canonical_url: Option<String>,
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

/// One run's worth of machine-readable provenance, kept separate from the
/// per-link records: which tool/version/API-versions produced this file
/// and under what settings, so `report.json` is self-describing evidence
/// rather than an opaque array a reader has to take on faith.
#[derive(Serialize, Debug)]
pub struct RunSummary {
    pub ghlinks_version: &'static str,
    pub github_api_version: &'static str,
    pub hacker_news_api: &'static str,
    pub reddit_note: &'static str,
    pub started_at: String,
    pub finished_at: String,
    pub input_file: String,
    pub total_urls: usize,
    pub link_kind_counts: BTreeMap<String, usize>,
    pub records_with_errors: usize,
    pub concurrency: usize,
    pub delay_ms: u64,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub skip_external: bool,
}

#[derive(Serialize, Debug)]
pub struct Report {
    pub schema_version: u32,
    pub run_summary: RunSummary,
    pub records: Vec<LinkRecord>,
}