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
/// shape (never emitted with an explicit `schema_version` field — its
/// absence identifies it). 2 is the wrapped `{schema_version, run_summary,
/// records}` shape introduced in 0.14 — see
/// `ADRs/wrap-report-json-output-in-schema-versioned-envelope.md` for the
/// full rationale and the alternatives considered.
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
    /// Count of releases whose `published_at` falls within the last 365
    /// days of when this record was fetched. Computed independently of
    /// `recent_releases`' ordering/bound — see that field's doc-comment for
    /// why the two are not the same collection and must not be conflated.
    pub releases_last_12_months: Option<i64>,
    /// The `tag_name` of `recent_releases[0]`, i.e. the first entry in
    /// GitHub's `CREATED_AT DESC` release ordering — NOT independently
    /// verified as the most-recently-*published* release. GitHub's
    /// GraphQL `ReleaseOrderField` only supports ordering by `CREATED_AT`
    /// or `NAME`, not `PUBLISHED_AT`, so "latest" here means "first by
    /// creation order," which is usually but not provably the same
    /// release as "most recently published" (a maintainer could, in
    /// principle, create a release for an older tag after a newer one
    /// already exists). Treat this as a `CREATED_AT`-ordering fact, not an
    /// independently-verified publish-date fact.
    pub latest_release_tag: Option<String>,
    /// `published_at` of `recent_releases[0]` — see `latest_release_tag`'s
    /// doc-comment for the same `CREATED_AT`-vs-`published_at` caveat.
    pub latest_release_published_at: Option<String>,
    /// Up to 100 releases, in the exact order GitHub's GraphQL API returns
    /// them for `orderBy: {field: CREATED_AT, direction: DESC}` (see
    /// `github.rs::releases()`). This is a **bounded recent-release
    /// listing, not a 12-month subset** — a repo with 3 releases in the
    /// last year and 97 older ones will still show up to 100 entries here,
    /// most of them outside any 12-month window. Use
    /// `releases_last_12_months` for the 365-day count; do not infer it
    /// from this list's length or contents.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal, deliberately sparse Report — enough to serialize, not
    /// meant to represent a realistic run. Kept in one place so every
    /// structural test below builds on the same known-good starting point.
    fn minimal_report() -> Report {
        Report {
            schema_version: SCHEMA_VERSION,
            run_summary: RunSummary {
                ghlinks_version: "0.0.0-test",
                github_api_version: "2022-11-28",
                hacker_news_api: "test",
                reddit_note: "test",
                started_at: "2026-01-01T00:00:00Z".into(),
                finished_at: "2026-01-01T00:00:01Z".into(),
                input_file: "links.txt".into(),
                total_urls: 1,
                link_kind_counts: BTreeMap::new(),
                records_with_errors: 0,
                concurrency: 1,
                delay_ms: 0,
                timeout_secs: 1,
                max_retries: 1,
                skip_external: false,
            },
            records: vec![],
        }
    }

    /// Guards the exact contract documented in the README and depended on
    /// by ADRs/wrap-report-json-output-in-schema-versioned-envelope.md: a
    /// consumer reads `.schema_version`, `.run_summary`, and `.records` at
    /// the top level, not a bare array. If this test needs to change, the
    /// README and that ADR need to change with it — not the other way
    /// around.
    #[test]
    fn report_serializes_as_an_object_with_the_three_documented_top_level_keys() {
        let value = serde_json::to_value(minimal_report()).unwrap();
        let obj = value.as_object().expect("Report must serialize as a JSON object, not an array");
        assert!(obj.contains_key("schema_version"));
        assert!(obj.contains_key("run_summary"));
        assert!(obj.contains_key("records"));
        assert_eq!(obj.len(), 3, "unexpected extra or missing top-level key");
    }

    #[test]
    fn serialized_schema_version_matches_the_schema_version_constant() {
        let value = serde_json::to_value(minimal_report()).unwrap();
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
    }

    #[test]
    fn records_array_length_matches_input() {
        let mut report = minimal_report();
        report.records.push(LinkRecord {
            input_url: "https://github.com/o/r".into(),
            canonical_url: Some("https://github.com/o/r".into()),
            link_kind: "repo_root".into(),
            owner: Some("o".into()),
            repo: Some("r".into()),
            file_path: None,
            repo_data: None,
            gist_data: None,
            pages_candidates_checked: vec![],
            pages_resolved_repo: None,
            external_mentions: vec![],
            external_discovery: ExternalDiscovery {
                skipped: true,
                sources: vec![],
                coverage: "test",
                hacker_news_query: "test",
                hacker_news_status: "skipped",
                hacker_news_mention_count: 0,
            },
            fetch_errors: vec![],
            fetched_at: "2026-01-01T00:00:00Z".into(),
            collector_version: "0.0.0-test",
        });
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["records"].as_array().unwrap().len(), 1);
    }

    /// The absence-vs-zero distinction `ExternalDiscovery`'s own
    /// doc-comment insists on is only real if it survives serialization —
    /// this locks in the actual wire values, not just the in-memory type.
    #[test]
    fn external_discovery_status_values_round_trip_through_json_as_documented() {
        let discovery = ExternalDiscovery {
            skipped: false,
            sources: vec!["hacker_news"],
            coverage: "test",
            hacker_news_query: "test",
            hacker_news_status: "ok",
            hacker_news_mention_count: 0,
        };
        let value = serde_json::to_value(&discovery).unwrap();
        assert_eq!(value["hacker_news_status"], "ok");
        assert_eq!(value["hacker_news_mention_count"], 0);
    }
}