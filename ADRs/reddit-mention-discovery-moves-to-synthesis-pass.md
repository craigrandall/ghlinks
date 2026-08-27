---
status: "accepted"
date: 2026-08-24
decision-makers: Craig
consulted: Claude (Anthropic) — technical research on Reddit API policy changes and implementation options
informed: n/a — single-developer project
---

# Discontinue ghlinks' direct Reddit API integration; delegate Reddit mention discovery to the Claude synthesis pass

## Context and Problem Statement

`ghlinks` is a local Rust CLI that deterministically collects structured facts about GitHub-hosted URLs (repo metadata via GitHub's GraphQL API, contributor counts, and external-mention discovery via Hacker News and Reddit) into a single `report.json`. That file is then handed to Claude for a single, efficient synthesis pass: plain-language summarization and sentiment characterization of how the linked content has been received externally.

`discovery::reddit()` in `discovery.rs` queries Reddit's unauthenticated `https://www.reddit.com/search.json` endpoint. That endpoint has stopped working: Reddit closed self-service OAuth app creation on 2025-11-11 under its "Responsible Builder Policy" (new client credentials for a personal, non-commercial project now require manual approval, with no published SLA and reportedly low approval odds for hobbyist/research use cases), and then killed the unauthenticated `.json` fallback entirely on 2026-05-30, which now returns HTTP 403.

Concretely, this means every current `ghlinks` run:
- Accumulates a `"reddit: Reddit HTTP 403"` entry in `fetch_errors` for every URL processed.
- Reports `"reddit"` in `ExternalDiscovery.sources` and describes a 15-result Reddit search in `coverage`, even though that search never actually executes.

A parallel, OAuth-based `reddit_client.rs` module (implementing Reddit's `client_credentials` application-only grant) was built as a prospective replacement, but requires a `client_id`/`client_secret` pair that isn't obtainable through the same request-and-wait approval process, on no predictable timeline.

The question this ADR resolves: **given Reddit's Data API is no longer viably self-serve for a personal, non-commercial project, where should Reddit mention-discovery live, and should `ghlinks` carry an authenticated Reddit client at all?**

## Decision Drivers

* `report.json` correctness — a structured field should never claim to represent a search that didn't run; this is the whole reason facts are sourced from APIs rather than inferred.
* Availability — Reddit's official Data API is gated behind a manual, low-SLA approval process for new non-commercial clients; there is no reliable path to credentials today.
* An existing architectural principle for this project: deterministic, factual retrieval (license, releases, contributor counts, etc.) belongs in `ghlinks`/API calls; reading-comprehension and characterization tasks belong to the LLM synthesis step.
* Avoid shipping or maintaining a second, credential-blocked Reddit-discovery code path alongside the dead unauthenticated one.
* Minimize new engineering surface area — scraping workarounds carry their own fragility (obfuscated frontend internals, proxy rotation) and Reddit ToS exposure.
* Preserve the ability to reinstate deterministic Reddit coverage later at low cost, without having thrown the OAuth implementation away.

## Considered Options

* Integrate `reddit_client.rs` (OAuth `client_credentials`) into `ghlinks` now, pending future credential approval
* Leave the existing unauthenticated `discovery::reddit()` call in place as-is
* Move Reddit mention discovery out of `ghlinks` into the Claude synthesis pass (web search at synthesis time)
* Drop Reddit coverage from the pipeline entirely
* Replace the API call with a scraping-based fallback (DIY or third-party scraping service) inside `ghlinks`

## Decision Outcome

Chosen option: **"Move Reddit mention discovery out of `ghlinks` into the Claude synthesis pass,"** because it is the only option that restores `report.json` correctness immediately, requires no new infrastructure, credentials, or ongoing cost, and does not commit engineering effort toward either a currently-unobtainable OAuth grant or a scraping workaround with its own maintenance burden and ToS risk. `reddit_client.rs` is retained, unwired from the build, as a low-cost reinstatement path if Reddit access is ever approved.

### Consequences

* Good, because `fetch_errors` and `ExternalDiscovery` in `report.json` no longer misrepresent Reddit as having been searched.
* Good, because no new engineering effort, dependency, credential, or recurring cost is introduced.
* Good, because Hacker News discovery (Algolia-backed, still functioning) is entirely unaffected.
* Good, because `reddit_client.rs` remains available, correct, and ready to wire back in at near-zero cost if Reddit's Data API access is ever granted.
* Bad, because Reddit mention discovery is no longer deterministic — search results can vary run-to-run and offer no guarantee of completeness, unlike an API-backed query.
* Bad, because structured fields the Reddit API would have supplied (score, comment count, exact `created_utc`) are not available from ad hoc web search results.
* Neutral, because this is a deliberate, scoped exception to the "structured facts vs. comprehension" split for exactly one signal source (Reddit mentions), not a change to that principle generally.

### Confirmation

Implementation is compliant when:
- `discovery.rs` no longer contains a function that calls `reddit.com`.
- `main.rs`'s `ExternalDiscovery` construction no longer lists `"reddit"` in `sources`, no longer references a Reddit-specific result limit, and `coverage` text describes only Hacker News.
- `model.rs`'s `ExternalDiscovery` struct no longer carries a Reddit-specific field, and the doc comment on `ExternalMention.source` no longer lists `"reddit"` as an expected value.
- A `ghlinks` run against a real URL produces a `report.json` with zero Reddit-related entries in `fetch_errors`.
- `reddit_client.rs` exists in the repository but outside `src/` (e.g. under a `future/` directory) and is not referenced by any `mod` declaration, so it is excluded from the compiled binary. **Status as of v0.14: not present in this repository** — the file was written before this decision but never committed. If it's still available locally, add it under `future/reddit_client.rs` (untracked by any `mod` declaration) to make this bullet true; if it's no longer available, drop this bullet and rely on the other four confirmation criteria, which don't depend on the file's presence.

## Pros and Cons of the Options

### Integrate `reddit_client.rs` into `ghlinks` now

* Good, because the OAuth mechanics are already implemented and correct.
* Good, because, if it worked, it would restore fully deterministic, structured Reddit data.
* Bad, because it requires a `client_id`/`client_secret` that cannot currently be obtained through Reddit's self-service flow.
* Bad, because it ships a code path that cannot run today, on an unknown timeline — the same "dead path" problem as the current state, just relocated behind an access gate instead of a dead endpoint.

### Leave the existing unauthenticated call as-is

* Bad, because it is provably broken (HTTP 403 since 2026-05-30) with no prospect of self-resolving.
* Bad, because it actively corrupts `report.json`'s discovery metadata on every run.
* Neutral, because it requires no engineering effort — but that's the only thing it has going for it.

### Move Reddit discovery to the Claude synthesis pass

* Good, because it requires no credentials, infrastructure, or new dependencies.
* Good, because it immediately eliminates the false-positive "reddit" source claim and 403 errors from `report.json`.
* Good, because it fits naturally into the synthesis pass, which already reads comprehensively and characterizes tone from unstructured sources.
* Bad, because it is not deterministic and does not return the structured fields (score, comment count, timestamp) the API would have.
* Bad, because it is a scoped exception to the project's deterministic-facts-vs-comprehension split.

### Drop Reddit coverage entirely

* Good, because it is the simplest possible fix and removes all Reddit-related complexity.
* Bad, because it discards real, if long-tail, signal — Reddit surfaces discussion (particularly in tool/subreddit-specific communities such as r/LocalLLaMA, r/MachineLearning) that doesn't reliably overlap with Hacker News coverage.
* Bad, because "drop" and "delegate to synthesis" cost roughly the same effort in `ghlinks` (both require removing the same dead call), so dropping the source captures none of the coverage for the same code change.

### Replace with a scraping-based fallback

* Good, because it doesn't depend on Reddit's OAuth approval process.
* Bad, because Reddit's terms restrict scraping that bypasses the official API.
* Bad, because community reports describe Reddit's rendered frontend as difficult to scrape reliably (obfuscated data transport), requiring proxy rotation and ongoing maintenance.
* Bad, because it is disproportionate engineering investment for a long-tail, not load-bearing, signal source.

## More Information

Two Reddit policy changes motivate this ADR and are worth recording for future reference, since either could change the calculus:

- 2025-11-11 — Reddit closed self-service OAuth app creation under its "Responsible Builder Policy," gating all new API client credentials behind manual, unpredictable-timeline approval.
- 2026-05-30 — Reddit disabled the unauthenticated `.json` search endpoint that `ghlinks` was using, returning HTTP 403 across the board.

Filing Reddit's official developer-access request ticket remains a zero-cost, parallel action outside the scope of this ADR — if approved, `reddit_client.rs` can be wired back into `ghlinks` at low cost, and this decision should be revisited at that point.

This decision should also be revisited if search-based Reddit discovery in the synthesis pass proves materially unreliable or insufficiently thorough in practice.
