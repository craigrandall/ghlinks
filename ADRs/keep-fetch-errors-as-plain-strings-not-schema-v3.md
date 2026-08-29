---
status: "accepted"
date: 2026-08-28
decision-makers: Craig
consulted: Claude (Anthropic) — see docs/failure-taxonomy-notes.md, the empirical prerequisite that informed this decision
informed: n/a — single-developer project
---

# Keep `fetch_errors` as `Vec<String>`; do not pursue a structured error schema (closing #9)

## Context and Problem Statement

Since the v0.13.1 research-quality audit, `fetch_errors: Vec<String>` was
flagged as "functional but weak for downstream research" — machine
consumers would need to parse free-text strings to distinguish, say, a
retryable 5xx from a permanent classification failure. The audit
explicitly cautioned against making this the next change "unless we're
ready to revise the schema again," and `docs/failure-taxonomy-notes.md`
was built as the empirical prerequisite: real, test-derived data on what
failures actually occur and how, gathered specifically so a future
schema-v3 decision wouldn't be made ahead of evidence.

That prerequisite is now satisfied. The question this ADR answers: **now
that the evidence exists, should `ghlinks` actually adopt a structured
error schema (`schema_version: 3`), or keep the current plain-string
approach?**

## Decision Drivers

* `ghlinks`'s only current consumer of `report.json` is an LLM-driven
  synthesis pass — not a strict machine parser, dashboard, or
  auto-retry script.
* The Pages-resolution failure-artifact work (v0.14.7) demonstrated that
  richly-worded plain strings can carry substantial machine-actionable
  content (which URL, which candidates were ruled out, a concrete next
  step) without any schema change.
* A `schema_version` bump is explicitly costly per
  `ADRs/wrap-report-json-output-in-schema-versioned-envelope.md`'s own
  policy — it is not a mechanical string-to-object conversion here: real
  open design questions remain unresolved in
  `docs/failure-taxonomy-notes.md` (severity per origin, which origins
  are "retryable," whether HN zero-results belongs in a taxonomy at all).
* Avoiding speculative complexity ahead of a concrete, demonstrated need
  is a explicit project value (see this decision's own framing: "perfect
  is the enemy of good enough / fit for purpose").

## Considered Options

* Design and ship a structured `fetch_errors` schema (`schema_version: 3`)
* Keep `fetch_errors: Vec<String>` as-is, with richer per-origin message
  content where it earns its keep (as already done for Pages resolution)

## Decision Outcome

Chosen option: **keep `fetch_errors: Vec<String>`; do not pursue schema
v3 at this time.** No consumer of `report.json` today needs structured
error querying, and the LLM synthesis pass — the tool's actual and only
downstream consumer — is well served by descriptive strings, as
demonstrated directly by the Pages-resolution enrichment. Designing a
structured error taxonomy now would mean resolving real open questions
(severity, retryability per origin) speculatively, against a consumer
that doesn't need them, rather than being driven by an actual concrete
requirement.

This decision is revisitable, not permanent: `docs/failure-taxonomy-notes.md`
remains a complete, accurate reference of empirically-observed failure
behavior, and is exactly the input a future schema-v3 effort would still
need — nothing about closing this decision now discards that work.

### Consequences

* Good, because it avoids schema churn (and the corresponding required
  updates to the envelope ADR, README examples, `model.rs`'s T-5
  structural tests, and this project's docs) for a need that doesn't
  currently exist.
* Good, because it keeps `schema_version: 2` stable, so nothing consuming
  `report.json` today needs to change.
* Good, because `docs/failure-taxonomy-notes.md` stays useful as a
  finished reference document (what actually happens, per origin) rather
  than becoming stale scaffolding for a decision that never got made.
* Bad, because if a future consumer *does* need structured
  filtering/aggregation (an auto-retry script, a dashboard, a monitoring
  integration), that need is deferred rather than already met — this
  will cost a future schema-v3 effort that could have been done now
  instead.
* Neutral, because per-origin message content can continue improving
  (as it did for Pages resolution) without this decision needing to be
  revisited — enriching a string's content is not a schema change.

### Confirmation

This decision is honored as long as:
- `fetch_errors` in `model.rs` remains `Vec<String>` and `schema_version`
  remains `2`, until a concrete downstream requirement — not a
  speculative one — motivates reopening this decision.
- Any future reopening starts from `docs/failure-taxonomy-notes.md`'s
  existing table rather than re-deriving the taxonomy from scratch.

## More Information

Reopen this decision when a concrete consumer need for structured errors
exists — e.g., an automated re-run/retry tool that needs to distinguish
retryable from permanent failures programmatically, or a non-LLM
consumer of `report.json`. At that point, `docs/failure-taxonomy-notes.md`
is the starting input for designing the shape, not a re-litigation of
whether to do it at all.
