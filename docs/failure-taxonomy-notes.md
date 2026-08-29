# Failure taxonomy notes (empirical input to the #9 decision — now closed)

**Status: informational reference, not a schema change — and not
pending one.** `fetch_errors` remains `Vec<String>` in `schema_version:
2`. This document was built as the empirical prerequisite for deciding
whether a structured `fetch_errors` schema (schema v3) was warranted:
define the domain-level failure taxonomy from *observed* behavior before
designing a schema around it. That decision has now been made — see
`ADRs/keep-fetch-errors-as-plain-strings-not-schema-v3.md`: **no schema
change, `fetch_errors` stays `Vec<String>`.** This document remains
useful as an accurate reference of what actually happens, per failure
origin, and is the starting input if that decision is ever reopened
against a concrete future need — see `docs/CONTRIBUTING.md` §5 and
`ADRs/wrap-report-json-output-in-schema-versioned-envelope.md`'s
schema-versioning policy for the surrounding context.

## What the T-3/T-4/T-7 tests actually established

Each row below is grounded in a specific test in
`src/github.rs::http_boundary_tests` or `src/main.rs::orchestration_tests`,
not a theoretical taxonomy invented ahead of the evidence.

| Origin | Observed `fetch_errors` behavior | Retried before surfacing? | Record still usable? |
|---|---|---|---|
| Classification (`Unknown`) | `"could not classify URL (unrecognized host/path shape)"` | n/a — no HTTP involved | Yes — `link_kind: "unknown"`, all other fields absent/empty |
| Out-of-scope but recognized (`UnsupportedGithubUrl`, `UserOrOrgProfile`) | Fixed descriptive string per kind | n/a | Yes — recognized and described, just not collected |
| GraphQL-level error in a 200 response | `"graphql_repo: GraphQL returned errors: {joined messages}"` | No — this is not an HTTP-status-based retry condition, see `retry::should_retry` | No — `repo_data` is `None` |
| Malformed/non-JSON response body | `"graphql_repo: parsing graphql repository json: ..."` (via `anyhow::Context`) | No | No |
| Plain HTTP 403 (no rate-limit signal) | `"...: repository existence check HTTP 403"` / equivalent per endpoint | No — proven not retried (`plain_403_is_reported_after_exactly_one_attempt`) | Depends on which call failed |
| HTTP 404 on `repo_exists` specifically | **Not an error at all** — `Ok(false)`, a valid answer for that endpoint | n/a | Yes — this is the one endpoint where 404 is meaningful data, not failure |
| HTTP 5xx / 429 / rate-limited 403 | Retried up to `--max-retries` attempts, then reported with the final status if still failing | Yes, exponential backoff (`retry.rs`) | Depends on which call failed |
| Pages: neither candidate resolves | Enriched message naming the original URL, both ruled-out candidates, and a concrete web-search query shape for the downstream LLM synthesis pass to act on (see `pages_unresolved_message()`) — no longer a fixed generic string | Each candidate check follows the normal per-request retry policy above | Yes — `pages_resolved_repo: null` with the explicit "unresolved, not none" semantics documented in `model.rs`; `pages_candidates_checked` entries are now annotated with each candidate's outcome (`exists` / `not found` / `check failed: ...`) |
| HN discovery failure | `"hacker_news: {error}"`, `external_discovery.hacker_news_status: "error"` | Yes, HN's own fixed retry budget (`discovery.rs::HN_MAX_RETRIES`) | Yes — GitHub data survives independently; this is the isolation `main.rs`'s module doc-comment describes |
| One link's failure (any origin above) | Isolated to that `LinkRecord`'s own `fetch_errors` | n/a | The rest of the batch is provably unaffected — see `run_batch` and `one_failing_link_does_not_abort_the_rest_of_the_batch` |

## What this does NOT yet resolve

This table describes *what happens today*. It intentionally does not
resolve the following — not because they were overlooked, but because
`ADRs/keep-fetch-errors-as-plain-strings-not-schema-v3.md` decided these
don't need resolving unless a concrete future need reopens that decision:

- Which of the rows above would be `error` vs. `warning` severity in a
  structured schema.
- Which would be `retryable` from a downstream consumer's perspective
  (e.g. should a research pipeline re-run `ghlinks` on records where the
  origin was a 5xx, but not on ones where it was `Unknown`
  classification?).
- Whether "HN zero results" (`hacker_news_status: "ok"`,
  `hacker_news_mention_count: 0`) would belong in any error taxonomy at
  all — current behavior treats it correctly as *not* an error, and that
  should stay true regardless.
- Whether GitHub 404 on endpoints *other than* `repo_exists` (where it's
  currently just "not success" and bails) would deserve its own distinct
  category, since it's a different situation from a repo that's private,
  rate-limited, or genuinely erroring.

## If #9 is ever reopened

`ADRs/keep-fetch-errors-as-plain-strings-not-schema-v3.md` closed this
question for now: no schema change. If a concrete future need (not a
speculative one) reopens it, use the table above as the starting input,
not the final answer: assign kind/severity/retryable values deliberately
(per link_kind and per failure origin above), get that taxonomy reviewed
on its own terms, and only then design the JSON shape and bump
`schema_version`. Changing `Vec<String>` -> `Vec<ErrorObject>` before the
taxonomy itself is settled risks exactly the kind of schema churn
`ADRs/wrap-report-json-output-in-schema-versioned-envelope.md` is meant to
avoid.
