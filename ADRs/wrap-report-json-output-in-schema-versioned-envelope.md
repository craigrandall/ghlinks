---
status: "proposed"
date: 2026-08-27
decision-makers: Craig
consulted: Claude (Anthropic) — drafted from prior engineering-review notes and current README content
informed: n/a — single-developer project
---

# Wrap `report.json`'s top-level output in a schema-versioned envelope instead of a bare array

## Context and Problem Statement

`ghlinks` is a high-integrity evidence collector: its entire value proposition rests on downstream consumers (a human, a script, or an LLM synthesis pass) being able to trust `report.json` without independently re-verifying how it was produced. Earlier versions of the tool emitted a bare JSON array of link records as the top-level output — `[ {...}, {...}, ... ]` — with no accompanying description of what produced it, what settings were in effect, or how many records succeeded versus failed.

As the tool grew (retry/backoff logic, proactive rate-limit throttling, configurable `--timeout-secs`/`--max-retries`, and a widening set of `link_kind` classifications), the gap between "what actually happened during this run" and "what `report.json` says happened" grew with it. A consumer reading a bare array has no way to tell, for example, whether external-mention discovery was skipped (`--skip-external`), how many of the input URLs failed to resolve, or which version of `ghlinks` and the GitHub API produced the file. A structured evidence collector that can't self-describe its own provenance undermines its own premise.

The question this ADR resolves: **should `report.json`'s top-level shape carry its own provenance and run metadata, and if so, in what form?**

## Decision Drivers

* Provenance and trust — an evidence-collection tool should not produce evidence that can't be checked for how and under what conditions it was gathered.
* Auditability — a consumer (especially an LLM synthesis pass) should be able to see aggregate run health (error counts, per-`link_kind` counts, whether external discovery ran) without scanning every record.
* Forward compatibility — future schema changes need an explicit, machine-checkable way to signal a breaking change to consumers, rather than relying on consumers noticing a shape change informally.
* Minimizing "quiet" breaking changes — consistent with the project's "honest failure reporting" principle, a future schema change should be visible and versioned, not silent.
* Backward-compatibility cost — wrapping the array is itself a breaking change for any existing consumer written against the bare-array shape; this cost is one-time and should be paid deliberately, not incrementally.
* Consistency with the "everything to one JSON file" design goal stated in the README — the fix should not require consumers to correlate two separate output files.

## Considered Options

* Keep the bare top-level array as-is (status quo)
* Wrap output in an object with `schema_version` and `records` only, omitting run-level metadata
* Wrap output in an object with `schema_version`, `run_summary`, and `records` (chosen)
* Emit two separate files — `report.json` (records) and `run_summary.json` (metadata) — read together
* Append a single synthetic "summary record" as an extra entry within the existing array

## Decision Outcome

Chosen option: **wrap output in an object with `schema_version`, `run_summary`, and `records`**, because it is the only option that gives consumers both a machine-checkable version signal and full run provenance (tool/API versions, settings, aggregate counts) in the single output file the project is designed around, without requiring a second file to be read and correlated.

### Consequences

* Good, because `report.json` is now self-describing: `run_summary` records `ghlinks_version`, `github_api_version`, the settings in effect (`concurrency`, `delay_ms`, `timeout_secs`, `max_retries`, `skip_external`), and aggregate outcomes (`total_urls`, `link_kind_counts`, `records_with_errors`) without requiring a full scan of `records`.
* Good, because `schema_version` gives any consumer — especially an LLM synthesis pass reading many `report.json` files over time — an explicit, checkable signal before assuming a particular shape.
* Good, because it keeps the "everything to one JSON file" property from the README intact; the two-file option would have broken that.
* Bad, because this is a breaking change: anything reading the old bare-array shape must be updated to read `.records[i]` instead of `data[i]`.
* Neutral, because the migration cost is one-time and is already reflected in the README's "Output shape" section and example.

### Confirmation

Implementation is compliant when:
- Every `report.json` produced by the tool is a top-level JSON object (not array) containing `schema_version`, `run_summary`, and `records` keys.
- `run_summary` includes at minimum: `ghlinks_version`, `github_api_version`, `started_at`/`finished_at`, `input_file`, `total_urls`, `link_kind_counts`, `records_with_errors`, and the run's effective CLI settings (`concurrency`, `delay_ms`, `timeout_secs`, `max_retries`, `skip_external`).
- Any future change to the shape of `records` entries or `run_summary` that isn't purely additive increments `schema_version`.
- README's "Output shape" section stays in sync with the actual emitted structure (it currently documents `schema_version: 2`).

## Pros and Cons of the Options

### Keep the bare array as-is

* Good, because no migration cost for any existing consumer.
* Bad, because it provides no provenance — a consumer cannot tell what produced the file or under what settings.
* Bad, because "zero results" and "collection failed" are indistinguishable without per-record inspection, undermining the project's honest-failure-reporting goal at the whole-run level.

### Wrap with `schema_version` + `records` only (no `run_summary`)

* Good, because it solves the versioning problem with a smaller change.
* Bad, because it leaves the core motivating problem — no run-level provenance — unsolved; a consumer still can't tell what settings produced the file or how many records failed without scanning all of them.

### Wrap with `schema_version`, `run_summary`, and `records` (chosen)

* Good, because it solves both the versioning and provenance problems in one shape.
* Bad, because it is the largest of the "wrap" options in terms of new fields to define and keep in sync with actual run behavior.

### Two separate output files

* Good, because it cleanly separates "facts about the run" from "facts about the links," which could simplify each file's schema individually.
* Bad, because it breaks the "everything to one JSON file" design goal and requires consumers to open, correlate, and keep in sync two files instead of one.

### Synthetic summary record appended to the array

* Good, because it requires no top-level shape change — existing bare-array consumers keep working, mostly.
* Bad, because it conflates two different kinds of thing (a link record and run metadata) in the same array, which is exactly the kind of un-self-describing ambiguity this ADR exists to eliminate — a consumer would need out-of-band knowledge to know the last (or first) array entry is "special."

## More Information

`schema_version` starts at `2`, not `1`. The original bare-array output is treated as the implicit, unversioned "version 1" shape; `2` marks the first shape that carries an explicit version number at all. There is no `schema_version: 1` ever emitted by the tool — encountering a `report.json` without a `schema_version` field at the top level should be read as "predates this ADR," not as a valid version 1 payload to branch on.

This decision should be revisited if a future schema change is large enough that maintaining both old and new consumers' expectations under a single `records` array becomes impractical — at that point, reconsider the "two separate files" option this ADR rejected.
