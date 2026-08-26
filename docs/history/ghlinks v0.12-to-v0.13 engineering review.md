# ghlinks v0.12-to-v0.13 engineering review

*Historical record — see `docs/ARCHITECTURE.md` and `docs/CONTRIBUTING.md`
for current-state documentation. This document is preserved for the
design rationale behind the v0.13 rewrite; it is not kept in sync with
later changes.*

**Requestor**: Craig Randall
**Reviewer**: Claude (free)
**Date**: Tue 8/25/2026
**External context note**: "v0.12" was separately used to seed https://github.com/craigrandall/ghlinks (i.e. its "Initial commit" content, including ADRs, docs, licenses, etc.)

Scope: a full read of `src/{classify,discovery,github,main,model}.rs` and
`run.ps1` as they existed at v0.12, cross-checked against your actual
baseline run (`verify-v0.12-research-quality-baseline-report.json`, 98
records) to ground findings in real behavior rather than hypotheticals.
Then a v0.13 implementation addressing what's practical to fix in one pass,
prioritized as you specified: **correctness → provenance → failure
visibility → tests → rate limiting.**

This document is the analysis and rationale. The literal diff is
preserved in git history as commit `d2464c4` ("ghlinks v0.13 engineering
review changes"); reproduce it locally with
`git diff a0828ba..d2464c4` against this repository.

---

## What's genuinely good here (not just "fine" — actually good design)

**The core separation of concerns is the single best decision in this
project**, and it's rarer than it should be: this tool resists the urge to
summarize, judge, or interpret anything. `fetch_errors` are recorded, not
swallowed; `external_mentions` reports raw scores and comment counts, not
"this was well-received." That restraint is what makes "high-integrity
evidence collector" a description of what the code already does, not
aspirational language layered on top of it in this pass.

**The GraphQL/REST split was already correct**, which is exactly the thing
you asked me to check rather than assume. A single GraphQL round-trip pulls
almost everything about a repo in one request instead of five; the
languages endpoint is called directly with no pagination logic (correct —
that endpoint doesn't paginate); releases *are* paged, correctly, via
GraphQL cursor (`pageInfo.hasNextPage` / `endCursor`) rather than trusting
the first page. Someone unfamiliar with GitHub's API surface gets at least
one of those wrong more often than not — this got both right.

**The contributors-count trick is genuinely clever**: rather than fetching
every contributor to count them, it requests `per_page=1` and reads the
page number out of the `Link: rel="last"` header — one request instead of
N, with a documented fallback (`github_contributors_count_semantics`) that
tells a reader exactly what the number does and doesn't mean. That
semantics field, and the equivalent `note` on `GistData` explaining why
star/fork counts are absent, are small things that show real care — a
caveat sitting right next to the number it qualifies gets read; the same
caveat in a README footnote doesn't.

**Per-record `fetch_errors`** (rather than one global error log) keeps
failures attributable to the specific link that caused them, which matters
enormously for an evidence collector — "something failed somewhere" is
much less useful than "this record, this call, this reason."

**`run.ps1` already did the secure-token thing right** before I touched
it — `Read-Host -AsSecureString` plus a `finally` block that zeros the
BSTR is genuinely careful code, not just "good enough." The output-path
bug you found doesn't reflect on that.

None of what follows is "this was badly built." It's "this was well built
for its first pass, and here's what a second pass earns you."

---

## Correctness

**Fixed: stringly-typed JSON traversal → typed models.** Every GraphQL and
REST response used to be walked as raw `serde_json::Value` —
`repo["defaultBranchRef"]["target"]["history"]["totalCount"]`-style chains
throughout `main.rs`. A typo in any one of those keys doesn't fail to
compile; it silently returns `Value::Null`, indistinguishable from the API
genuinely returning null. `github.rs` now defines real structs
(`RepositoryNode`, `ReleaseNode`, `GistResponse`, etc.) with
`#[serde(rename_all = "camelCase")]` mapping directly onto the existing
hand-written queries, so a field-name mismatch is a deserialization error
you see, not a `None` three call-sites downstream. I want to be direct
about the risk here: this is the largest, least-verified part of this
diff (see "Verification" below) — I traced every field access by hand
against the struct definitions and I'm confident in it, but "I'm
confident" and "the compiler confirmed it" are different claims, and I'm
not making the second one.

**Fixed: URL classification was too coarse.** Your own baseline data
proved this empirically — `github.com/eugeneyan/` and
`github.com/modelcontextprotocol` both landed in generic `"unknown"`,
indistinguishable from `github.blog/...`, which is a genuinely different
kind of failure (a host `classify()` doesn't recognize at all vs. a
GitHub URL it understands perfectly well and correctly excludes). There's
now a `UserOrOrgProfile` variant for the former case. This is exactly the
"recognized-but-unsupported vs. genuinely unknown" distinction you asked
for, and it was findable because you gave me a real run to check against
rather than a hypothetical one.

**Fixed: normalization was implicit, not recorded.** `classify()` already
*applied* normalization (trailing slash, `.git` suffix) but nothing
captured a canonical form anywhere. Each record now carries
`canonical_url`, produced by the same rules `classify()` uses internally,
so `github.com/Owner/Repo/` and `github.com/owner/repo.git` are both
traceable to the same normalized identity without re-deriving it.

**Confirmed correct, left alone:** REST languages endpoint usage, GraphQL
releases pagination. Re-implementing something that already works
correctly, just to have touched it, isn't correctness work — it's noise
in the diff.

---

## Reliability

**Fixed: no retry/backoff, at all, anywhere.** This was the single most
consequential reliability gap. A transient network blip, a GitHub 500, or
a secondary rate limit hit permanently failed that call for that URL —
one bad millisecond and a perfectly healthy repo shows up in
`fetch_errors` looking like something's wrong with it, when nothing was.
Across a 98-URL batch making several hundred HTTP calls, some of those are
going to be transient purely on probability grounds. `retry.rs` is a new,
dependency-free, pure-function module (no `rand` crate — jitter comes from
the system clock, which is enough to avoid synchronized retry storms
without adding a dependency I can't verify compiles here) that both the
GitHub client and Hacker News discovery now route every request through.

One correctness point I want to call out specifically: a plain HTTP 403 is
**not** always safe to retry — it's GitHub's status code for both "you've
hit a secondary rate limit" and "you genuinely don't have permission to
see this" (private repo, wrong token scope, blocked). Retrying the second
kind wastes time and looks like hammering a server that already said no.
`should_retry` only treats a 403 as retryable when it carries a
`Retry-After` header or an exhausted `X-RateLimit-Remaining: 0` — both
explicit GitHub signals — and leaves a bare 403 alone. This distinction is
covered by a dedicated test (`plain_forbidden_with_no_rate_limit_signal_is_not_retried`)
specifically because it's the kind of thing that's easy to get
overzealous about and quietly start retrying things that should fail fast.

**Fixed: no proactive rate-limit awareness.** The tool now reads
`X-RateLimit-Remaining`/`X-RateLimit-Reset` off every GitHub response and
proactively pauses before the *next* call if the window's nearly
exhausted, capped at a 15-minute wait so one link can't stall an entire
batch run indefinitely. This is header-driven rather than a hardcoded
quota number, deliberately — GitHub's actual limits vary by endpoint and
have changed before.

**Fixed: hardcoded, unconfigurable 30-second timeout →** `--timeout-secs`,
default unchanged.

**Partially addressed: test suite.** See the dedicated section below —
this is real but incomplete, and I want to be specific about the gap
rather than claim more coverage than exists.

---

## Provenance

**Fixed: no record of what tool/API/query versions produced this data.**
The output's biggest structural change: the top-level shape moved from a
bare JSON array to `{schema_version, run_summary, records}`.
`run_summary` records `ghlinks_version`, `github_api_version` (the
`X-GitHub-Api-Version` header value actually sent), which HN API was used,
an explicit note that Reddit isn't queried (pointing at the ADR), run
timestamps, input file, per-`link_kind` counts, and every setting the run
was made with (concurrency, delay, timeout, retries). A `report.json`
that's just an array asks a reader to trust it; one that describes its own
provenance doesn't have to be trusted, it can be checked.

**This is a breaking schema change**, flagged prominently in the diff, in
`README.md`, and here. Anything downstream reading `report.json` — your
own synthesis-pass prompting included — needs to read `.records[i]`
instead of `data[i]` going forward.

---

## Failure visibility

**Fixed: "zero results" and "the search failed" were indistinguishable.**
Previously, if Hacker News search failed, `external_mentions` stayed
whatever it already was (empty, if nothing had been added yet) — visually
identical to a genuine zero-hits finding. The only way to tell them apart
was to separately scan `fetch_errors` for an `"hacker_news:"`-prefixed
string and cross-reference. `external_discovery` now carries
`hacker_news_status` (`"ok"` / `"error"` / `"skipped"`) and
`hacker_news_mention_count` explicitly, so "ran and found nothing" and
"didn't run successfully" are two different, checkable facts rather than
one ambiguous empty array.

**Retries and proactive pauses are visible in real time**, to stderr, as
they happen (`retry N/M after HTTP 429; waiting Xs`) — not silent, and not
bloating `report.json` with noise for retries that ultimately succeeded.

---

## Tests

Real additions, and I want to be precise about what they do and don't
cover:

- **Pure decision-logic tests** for retry/backoff (`retry.rs`, 11 tests):
  which statuses retry, the 403-with-signal-vs-without distinction, backoff
  growth, `Retry-After` precedence, proactive-wait capping. These need no
  HTTP at all — the policy is deliberately separated from the mechanism
  specifically so it's testable this way.
- **Through-serde deserialization tests** for every new typed struct
  (`github.rs`, 9 tests): a full realistic GraphQL repo response, a paged
  releases connection, a GraphQL errors array, a null repository, a gist
  response, missing-optional-fields tolerance. These exercise the exact
  risk surface I flagged above — the field-name mapping between the query
  and the Rust structs — using real JSON literals, not manual struct
  construction, so a rename mistake would actually be caught by a failing
  test rather than papered over.
- **Classification tests** (`classify.rs`, 6 tests) covering the new
  `UserOrOrgProfile` distinction and `canonicalize()`'s normalization rules
  explicitly.
- **Extended `main.rs` tests** (6 tests): the pre-existing release-counting
  tests rewritten against the new typed API, plus new coverage for
  gist-building and the profile/unknown distinction at the `describe_kind`
  level.

**What's not here, and why**: full HTTP-level integration tests — actually
standing up mock GitHub/HN servers and exercising `send_with_retry`'s real
network path — aren't included. That needs an HTTP-mocking crate
(`wiremock` is the natural choice), which is a new dependency I cannot
verify resolves or compiles in this sandbox (no network, no cargo — see
below). Adding an unverified dependency to a project I can't compile-check
is a worse trade than not adding it. What I did instead: added a
`GitHub::with_base_url()` hook so the client's target is injectable
without touching any call site, specifically so this is a short follow-up
for you to add locally, where you actually can verify it compiles. That's
a real gap, not a rounding error — the retry loop's actual HTTP behavior
(does a mocked 429 really get retried the right number of times against a
live `reqwest` call, not just against a synthetic `HeaderMap`) is untested
end-to-end. The pure-function tests give me confidence in the policy; they
don't prove the mechanism wired to real HTTP.

---

## Rate limiting

Covered above under Reliability — proactive header-driven throttling plus
retry-triggered backoff, applied uniformly to both GitHub and Hacker News
through the shared `retry.rs` policy. One nuance I did *not* handle
specially: GitHub's GraphQL rate limit is technically points-based, not a
flat per-request count, and a single complex query can cost more than one
point. `throttle_if_low` treats `X-RateLimit-Remaining` uniformly for both
REST and GraphQL, which is directionally correct (GitHub returns that
header on both surfaces) but doesn't account for variable query cost.
Unlikely to matter at this tool's actual scale (a few hundred links per
run) — worth knowing about if batch sizes grow much larger.

---

## Security / usability

- **`--token` is now hidden from `--help`**, not removed — existing
  scripts that pass it explicitly keep working, but the documented,
  advertised path is `$GITHUB_TOKEN` or `run.ps1`'s secure prompt. A token
  as a literal CLI argument is visible in shell history and process
  listings; that's the thing being discouraged, not the flag's existence.
- **`--timeout-secs` and `--max-retries`** are now real, documented,
  configurable flags rather than a buried constant.
- **The `run.ps1` output-path bug is fixed**: a relative `-OutputFile`
  (including the default) now always resolves against `-InputFile`'s own
  directory, never against whatever directory PowerShell happened to be
  launched from — which is what put your report in `C:\Windows\System32`
  when PowerShell opened "as Administrator" (that shell's default working
  directory). An explicit absolute path still overrides this, on purpose.
- **`run.ps1` is documented, explicitly, as a convenience wrapper** — the
  binary works standalone; the wrapper adds a secure prompt and a build
  step, nothing architectural.

---

## Research quality

Covered above under Provenance and Failure visibility — recorded
tool/API/query versions, zero-vs-failed distinction for HN. One thing
*not* changed: `github_contributors_count_semantics` and the gist `note`
field were already doing this well before I touched anything; I left them
as-is rather than "improve" something that wasn't broken.

---

## Performance

No structural change to the happy path — same concurrency/delay model,
same number of calls per link. Retries add latency specifically when
something's actually failing, which is the correct trade for a tool whose
job is evidence integrity, not speed. The proactive rate-limit pause is
the only thing that can add latency on a fully healthy run, and only once
you're within 2 requests of exhausting GitHub's window — not something a
normal-sized batch should ever hit.

---

## Recommendations considered and *not* implemented, with why

Per your instruction not to claim things that weren't done — this is the
honest list, not a "nice to have someday" wishlist dressed up as done
work:

1. **`wiremock`-based HTTP integration tests.** Not added — new
   dependency I can't verify compiles here (no network, no cargo).
   `GitHub::with_base_url()` is the prepared hook; this is a genuinely good
   next step for you specifically, where you can verify it.
2. **`rand` crate for backoff jitter.** Not added — unnecessary for this
   use case (spacing out retries doesn't need cryptographic randomness)
   and avoids a dependency I can't verify. Used the system clock instead.
3. **Threading `--max-retries` through to Hacker News too** (it currently
   uses its own fixed internal constant of 3). Not done, for plumbing-risk
   discipline — HN is a lower-stakes, best-effort supplementary signal, and
   threading one more parameter through the `stream::iter().map()` async
   closure in `main.rs` for comparatively low value felt like the wrong
   place to spend risk budget in an already-large diff. Trivial follow-up
   if you want it unified.
4. **Recording GitHub's *canonical* owner/repo casing** (from the GraphQL
   response itself, distinct from whatever casing the input URL used).
   Real potential value — URLs are inconsistently cased in the wild — but
   it's a genuine schema design decision (new field? overload `owner`?)
   that didn't belong buried inside an already-large diff. Good candidate
   for its own ADR if exact-casing dedup ever matters to you.
5. **Renaming `RepoFile`'s `branch` field** to something like `git_ref`,
   since a blob URL's ref can be a branch, tag, or full commit SHA, not
   only a branch. Not done, deliberately, because you explicitly said to
   preserve source terminology, and this touches both the enum and (if
   propagated) the JSON schema. Flagged as a naming-precision
   recommendation, not acted on.
6. **Incremental/resumable output** (writing records to disk as they
   complete, so a crash mid-batch doesn't lose already-collected work).
   Real resilience value for large batches, but it's a genuinely bigger
   architectural change — exactly the "bigger scraper" direction you said
   not to take this in. Worth an ADR if batch sizes grow large enough that
   losing a whole run to one crash becomes a real cost; not implemented
   here.
7. **GraphQL point-cost-aware rate limiting** (vs. the current uniform
   remaining-count check). Noted above under Rate limiting — not handled
   specially; unlikely to matter at current scale.

---

## Verification — what actually happened, not what should have

I want to be exact about this rather than imply more than occurred.

**This sandbox has no Rust toolchain, no PowerShell, and no network
access.** I checked directly before starting: `cargo`, `rustc`, and `pwsh`
are all absent (`command not found`), there's no cargo registry cache
anywhere on disk, and `curl` to both `crates.io` and `api.github.com`
returns `403` from the egress proxy. I ran the actual commands anyway to
confirm rather than assume:

```
cargo fmt --check     → cargo: not found
cargo check            → cargo: not found
cargo clippy            → cargo: not found
cargo test              → cargo: not found
cargo build --release   → cargo: not found
```

None of that ran. I'm not going to describe any of it as passing.

**What I actually did instead**, as the closest available substitute:

- Balanced every brace/paren in every changed file.
- Extracted every `struct` definition's field list programmatically and
  diffed it, field-for-field, in order, against every construction site
  (`RepoData`, `GistData`, `LinkRecord`, `ExternalDiscovery`, `RunSummary`,
  `Report`) — all matched exactly.
- Manually traced every `.as_ref().and_then(...)` chain in
  `build_repo_data`/`apply_releases`/`build_gist_data` against the actual
  `github.rs` struct shapes, field by field, confirming types line up
  through every `Option`/reference layer.
- Read every changed file in full at least once after writing it, looking
  specifically for the kind of mistake a compiler catches instantly and a
  human skims past — mismatched names, wrong `Option` handling, dropped
  fields.
- Confirmed zero new dependencies were added (checked `Cargo.toml`
  directly), which was a deliberate choice to keep this diff's
  unverified-by-me surface as small as possible.
- Generated a real `diff -ruN` against your v0.12 baseline rather than
  describing changes from memory (preserved today as git commit
  `d2464c4`; reproduce with `git diff a0828ba..d2464c4`).

That's a real, disciplined review — but it is categorically not the same
claim as "this compiles." Please treat `cargo fmt && cargo check && cargo
clippy --all-targets -- -D warnings && cargo test && cargo build
--release` as the actual verification gate, run it locally, and send me
whatever the compiler says. Given how much of this diff is new (five of
six source files rewritten, one new file), I'd genuinely be surprised if
there's zero fallout — a missing import or an `Option` handling mismatch
in code this size, unverified by a compiler, is a realistic outcome, not
a remote one. I'm ready to fix whatever surfaces immediately.

**On `run.ps1`**: the output-path logic is a small, self-contained
`Split-Path`/`Join-Path`/`IsPathRooted` change I traced by hand and am
confident in, but "traced by hand" isn't "ran it" — no `pwsh` here either.
Please run it for real as the actual check.

---

## Files changed

`classify.rs`, `discovery.rs`, `github.rs`, `main.rs`, `model.rs` (all
five source files), plus a new `retry.rs`, `run.ps1`, `README.md`,
`Cargo.toml`/`Cargo.lock` (version bump only). Full detail is in git
commit `d2464c4` (`git diff a0828ba..d2464c4`).
