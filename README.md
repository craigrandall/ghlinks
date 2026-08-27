# ghlinks

A deterministic collector for a list of GitHub-hosted links (repo roots,
files-in-a-repo, gists, GitHub Pages sites). It pulls structured facts from
GitHub's API and checks one free, keyless API (Hacker News) for external
mentions, and writes everything to one JSON file. It does **not**
summarize, interpret, or judge tone/sentiment — that's a job for a human or
an LLM reading the output, not for a scraper. It is deliberately a
**high-integrity evidence collector**, not a scraper: it would rather
report "I don't know" honestly than guess, retry silently forever, or
quietly drop a source without saying so.

Additional project documentation may be found in the `docs` folder (e.g.
`ARCHITECTURE.md` and the Mermaid diagrams (.mmd files) it points at, and
`CONTRIBUTING.md`). `docs/history/` holds point-in-time review documents
that are not kept current — see the note at the top of each.

## Why it's shaped this way

Fetching and reading rendered GitHub HTML pages to get facts like "license"
or "star count" is slow, token-expensive if an LLM is doing it, and
non-deterministic (results vary with page layout). GitHub's actual API
returns the same facts as clean JSON in a fraction of the size. This tool
does the mechanical lookup so that whatever reads its output — you, a
script, or an LLM — starts from ~300–800 tokens of clean facts per link
instead of ~10–40k tokens of scraped HTML.

## Requirements

- **Rust** (stable toolchain) — install via [rustup.rs](https://rustup.rs)
- **A GitHub personal access token**, with the least privileges GitHub
  currently permits for public metadata. The GraphQL
  endpoint this tool relies on for most fields requires *some* token; there
  is no unauthenticated GraphQL access.
- Internet access on first `cargo build` (to download crates) and every run
  (to hit GitHub/HN).

## Build & run

### Windows (PowerShell 7.x)

```powershell
$env:GITHUB_TOKEN = "ghp_..."          # or let run.ps1 prompt you securely
./run.ps1 -InputFile .\links.txt -OutputFile report.json
```

Useful flags: `-SkipExternal` (skip HN lookups), `-Concurrency 5`,
`-DelayMs 400` (be gentler on rate limits), `-TimeoutSecs 45`,
`-MaxRetries 5`, `-SkipBuild` (reuse an existing binary).

A relative `-OutputFile` (including the default) is always resolved
against `-InputFile`'s own directory, not whatever directory you happened
to launch PowerShell from — pass an absolute path if you deliberately want
the report written somewhere else.

`run.ps1` is a convenience wrapper, not the architecture — everything it
does (build, prompt for a token, invoke the binary) is optional sugar
around the actual program below. It exists mainly so a token never has to
be typed as a literal command-line argument.

### macOS / Linux / manual

```bash
export GITHUB_TOKEN=ghp_...
cargo build --release
./target/release/ghlinks --input links.txt --output report.json
```

Run `./target/release/ghlinks --help` for all flags, including
`--timeout-secs` and `--max-retries`.

`run.ps1` hides token entry, but passes the resulting token to the child
process through `GITHUB_TOKEN`; it is secure entry, not secure storage. Do
not use `-SkipBuild` unless you trust the existing project-local binary.

## Input format

Plain text, one URL per line. Blank lines and lines starting with `#` are
ignored.

## Output shape

The top-level output used to be a bare JSON array. As of 0.14
(`schema_version: 2`) it's a single object with a `run_summary` 
(provenance: tool/API versions, run settings, aggregate counts) alongside 
the `records` array — a bare array with no self -description made it easy 
for a consumer to trust `report.json` without any way to check what 
actually produced it. Update anything reading `report.json` to look at 
`.records[i]` instead of `data[i]`, and `.run_summary` for provenance.

```jsonc
{
  "schema_version": 2,
  "run_summary": {
    "ghlinks_version": "0.14.0",
    "github_api_version": "2022-11-28",
    "hacker_news_api": "hn.algolia.com/api/v1/search (Algolia-backed HN Search API)",
    "reddit_note": "Reddit is not queried by ghlinks; see ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md",
    "started_at": "2026-08-27T05:00:00Z",
    "finished_at": "2026-08-27T05:02:10Z",
    "input_file": "links.txt",
    "total_urls": 98,
    "link_kind_counts": { "repo_root": 75, "gist": 4, "pages_site": 6, "user_or_org_profile": 2, "unsupported_github_url": 2, "unknown": 3 },
    "records_with_errors": 7,
    "concurrency": 3,
    "delay_ms": 250,
    "timeout_secs": 30,
    "max_retries": 3,
    "skip_external": false
  },
  "records": [
    {
      "input_url": "https://github.com/owner/Repo.git/",
      "canonical_url": "https://github.com/owner/Repo",
      "link_kind": "repo_root",          // repo_root | repo_file | gist | pages_site | user_or_org_profile | unsupported_github_url | unknown
      "owner": "owner",
      "repo": "Repo",
      "file_path": null,                 // populated for repo_file links
      "repo_data": {
        "description": "...",
        "license_key": "mit",            // SPDX-ish key; null if GitHub couldn't detect one
        "license_name": "MIT License",
        "stargazers_count": 803,
        "forks_count": 97,
        "watchers_count": 2,
        "open_issues_count": 12,
        "closed_issues_count": 340,
        "primary_language": "TypeScript",
        "languages_bytes": { "TypeScript": 900000, "CSS": 12000 },
        "created_at": "2025-11-01T00:00:00Z",
        "pushed_at": "2026-03-10T00:00:00Z",
        "default_branch": "main",
        "commit_count_default_branch": 1240,
        "topics": ["ai-agents", "kanban"],
        "github_contributors_count": 14,
        "github_contributors_count_semantics": "GitHub contributors endpoint entries; anon=true; not unique humans",
        "releases_total_count": 61,
        "releases_last_12_months": 61,
        "latest_release_tag": "v6.1.0",
        "latest_release_published_at": "2026-03-15T00:00:00Z",
        "recent_releases": [ /* up to 100 most recent */ ]
      },
      "gist_data": null,
      "pages_candidates_checked": [],
      "pages_resolved_repo": null,
      "external_mentions": [
        {
          "source": "hacker_news",
          "title": "Show HN: ...",
          "url": "https://news.ycombinator.com/item?id=...",
          "score": 142,
          "num_comments": 38,
          "created_at": "2026-01-05T12:00:00Z"
        }
      ],
      "external_discovery": {
        "skipped": false,
        "sources": ["hacker_news"],
        "coverage": "Discovery signals only: HN stories linking to the exact URL. Comments and other sites are not searched by ghlinks — Reddit mentions are discovered during the downstream LLM synthesis pass instead (see ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md).",
        "hacker_news_query": "exact URL; stories only",
        "hacker_news_status": "ok",       // "ok" | "error" | "skipped" — "ok" + 0 mentions is a real zero-results finding; "error" means the search itself failed, see fetch_errors
        "hacker_news_mention_count": 1
      },
      "fetch_errors": [],
      "fetched_at": "2026-08-27T05:00:41Z",
      "collector_version": "0.14.0"
    }
  ]
}
```

## Architecture decisions

Significant, hard-to-reverse design choices are recorded as ADRs in
[`ADRs/`](ADRs/) (MADR format — see `ADRs/_README.md`). Read those for the
reasoning behind anything here that looks like it could have been done a
different way; this README documents *what* the tool does, the ADRs
document *why* the non-obvious choices were made.

## Known limitations (by design, not oversights)

- **Gist star/fork counts are not in this output.** GitHub's REST API
  doesn't expose total star/fork counts for gists (only whether *you*
  starred it) — only the gist's HTML page shows those numbers. Everything
  else about a gist (description, files, revision count, comments) is
  included.
- **GitHub Pages sites are resolved by guessing, then verifying.** A site at
  `owner.github.io/project` might be backed by `owner/owner.github.io` or by
  `owner/project` — the tool checks both and records which one (if either)
  actually exists as `pages_resolved_repo`. If neither exists (e.g. content
  lives in a subfolder, or the Pages site isn't repo-backed at all),
  `pages_resolved_repo` stays `null` and it's flagged in `fetch_errors` for
  manual follow-up.
- **A GitHub user/org profile page is recognized but not collected.**
  `link_kind: "user_or_org_profile"` (e.g. `github.com/{login}` with no
  repo segment) is deliberately distinct from `"unknown"` — it means the
  URL was understood and is simply out of this tool's scope, not that the
  URL couldn't be parsed at all.
- **License detection reflects GitHub's own linguist-based detector**, not a
  manual read of the LICENSE file. A repo can have a license file GitHub
  fails to classify; `license_key`/`license_name` will be `null` in that
  case rather than wrong.
- **Language breakdown and release rollup use supplemental API calls.** The
  language endpoint is used instead of GraphQL's capped language connection.
  Release pages are traversed so `releases_last_12_months` is based on every
  returned release, not only the first 100. Very release-heavy repositories
  can therefore take longer and consume more API quota.
- **Contributor count has GitHub endpoint semantics.** It is the count of
  entries returned by GitHub's contributors aggregation with `anon=true`, not
  a claim about unique human developers.
- **External-mention coverage from `ghlinks` itself is limited to Hacker
  News** — the only remaining source with a free, keyless, ToS-friendly
  search API. Reddit's Data API no longer supports self-service access for
  a personal, non-commercial client (see
  `ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md` for the full
  rationale); Reddit mentions are checked during the downstream LLM
  synthesis pass instead, not by this tool. It will not find mentions in
  blogs, X/Twitter, LinkedIn, Substack, or Discord either. Treat
  `external_mentions` as a floor, not a ceiling, on real-world discussion.
- **HTTP and discovery failures are recorded, and "zero" is never confused
  with "unknown."** A non-success HN response is never silently reported as
  zero mentions — `external_discovery.hacker_news_status` is `"ok"` only
  when the search actually ran; `"error"` means it failed and the reason is
  in `fetch_errors`. Check that status, not just whether
  `external_mentions` is empty, before treating a zero as a real finding.
- **Transient failures are retried; permanent ones are not.** GitHub and
  Hacker News calls automatically retry HTTP 429, 5xx, and 403 responses
  GitHub marks as rate-limit-related (via a `Retry-After` header or an
  exhausted `X-RateLimit-Remaining`), with exponential backoff — up to
  `--max-retries` attempts (GitHub calls only; Hacker News uses its own
  fixed budget). A plain 403 with no rate-limit signal (a genuine
  permission error) is not retried. The tool also proactively pauses
  before the next GitHub call if the rate-limit window is nearly
  exhausted, capped at a 15-minute wait so one link can't stall an entire
  batch run.
- **No sentiment/stance judgment.** The tool reports *that* something was
  posted and its raw score/comment count — not whether the discussion was
  supportive or critical. That read needs an actual reader.

## Intended workflow

1. Run this tool locally against your link list → `report.json`.
2. Hand `report.json` (plus the original link list, for anything the tool
   flagged as unresolved) to an LLM.
3. The LLM writes the actual report: plain-language project summaries, and
   — using the titles/scores/comments in `external_mentions` (Hacker News),
   plus targeted Reddit and other web searches to cover the ground ghlinks
   no longer does directly — the supportive/critical/concerned
   characterization the raw JSON can't produce on its own. `run_summary`
   tells the LLM (or you) exactly what ran, with what settings, so nothing
   about the evidence's provenance has to be assumed.

This keeps the deterministic, rerunnable part deterministic, and keeps the
judgment calls with whoever's actually reading the discussions.

---

## Contributing

Contributions are welcome. See `docs\CONTRIBUTING.md` for more details.
---

## License

Dual-licensed under MIT or Apache-2.0, at your option. See
`LICENSE-MIT` and `LICENSE-APACHE`.