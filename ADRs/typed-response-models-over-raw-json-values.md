---
status: "accepted"
date: 2026-08-27
decision-makers: Craig
consulted: Claude (Anthropic) — drafted from prior engineering-review notes
informed: n/a — single-developer project
---

# Decode GitHub GraphQL/REST responses into typed structs at the API boundary, instead of traversing raw `serde_json::Value`

## Context and Problem Statement

`ghlinks`'s GitHub integration (`github.rs`) calls both the GraphQL API (for most repo facts in one round-trip) and several REST endpoints (languages, contributor counts via `Link`-header pagination, gist data). Earlier code accessed fields from these responses by traversing `serde_json::Value` directly at or near the call sites — `.get("field").and_then(...)`-style chains with manual type coercion for each field consumed.

This pattern has real correctness risk for a tool whose stated top priority is correctness, ahead of provenance, failure visibility, tests, and rate limiting. A missing, renamed, or unexpectedly-typed field in a raw `Value` traversal tends to fail *softly* — coercing to `null`, a default, or simply not being read — rather than failing loudly at the point where the mismatch actually occurs. That's the opposite of the project's stated preference for honest failure over silent guessing. It also means the "shape" of what GitHub actually returns is implicit and scattered across call sites rather than centralized anywhere a future contributor (human or AI-assisted) could read it in one place.

The question this ADR resolves: **should GitHub API responses be decoded into typed Rust structs at the boundary, or continue to be traversed as raw JSON values at each call site?**

## Decision Drivers

* Correctness first — this project's explicit priority order puts correctness above provenance, failure visibility, tests, and rate limiting; a stringly-typed JSON traversal is a correctness risk relative to a typed one.
* Fail loudly, not softly — a missing or malformed field should surface as a deserialization error tied to a specific response, not silently become `null` or a default deep inside business logic.
* Centralized schema knowledge — one location describing "what GitHub actually returns" is more maintainable than the same knowledge implicitly encoded across many `.get(...)` call sites.
* Reviewability, including AI-assisted review — typed structs give any reviewer (human or AI, especially one without a compiler in the loop) a much smaller, checkable surface area than tracing dynamic JSON access paths by hand.
* Maintenance cost — typed structs require writing and updating `#[derive(Deserialize)]` types to track GitHub's actual response shapes, including handling fields GitHub might add, remove, or leave absent.

## Considered Options

* Keep raw `serde_json::Value` traversal at call sites (status quo)
* Introduce typed structs decoded immediately at the API boundary, before any business logic touches the data (chosen)
* Use a schema/code-generation approach (e.g., a `graphql_client`-style crate) to generate types directly from the GraphQL schema and query documents
* Hybrid: typed structs for core, frequently-used fields; raw `Value` passthrough for rarely-used or optional fields

## Decision Outcome

Chosen option: **typed structs decoded at the API boundary**, because it directly serves the project's top-priority correctness goal and its "honest failure" ethos — a shape mismatch becomes a `serde` deserialization error attached to the specific response that produced it, rather than a quietly wrong or missing value discovered later. It also gives `model.rs` and the response types a second role as living documentation of GitHub's actual response shape, which benefits both Craig and any AI-assisted review of future changes.

### Consequences

* Good, because malformed or unexpectedly-shaped responses fail at deserialization, with a clear error tied to the offending response, instead of surfacing later as a wrong or missing value in `report.json`.
* Good, because the response types double as documentation of what GitHub's API actually returns, reducing the need to re-derive that knowledge from scattered call sites.
* Good, because field renames or removals in the response-handling code become compiler-checked rather than discovered at runtime.
* Bad, because it requires more upfront code — a struct per response shape consumed — than ad hoc `Value` traversal.
* Bad, because GitHub API additions or changes require updating the corresponding struct, though liberal use of `#[serde(default)]`/`Option<T>` limits how often this causes a hard break.
* Neutral, because this pairs with the existing `retry.rs` pattern (pure, testable logic decoupled from I/O) as part of a broader "typed, testable boundaries" approach in the codebase, without itself constituting a new architectural principle.

### Confirmation

Implementation is compliant when:
- GraphQL and REST response bodies are deserialized into `#[derive(Deserialize)]` structs immediately upon receipt, before any field is read by business logic.
- No call site outside the deserialization boundary accesses a GitHub response via `serde_json::Value` indexing (`["field"]`) or `.get(...)`.
- A field GitHub omits or changes the type of on a real response causes a visible deserialization error (surfaced via `fetch_errors` for that link) rather than a silently wrong or default value in `report.json`.

## Pros and Cons of the Options

### Keep raw `serde_json::Value` traversal

* Good, because it requires no struct maintenance as GitHub's API evolves.
* Bad, because it is the correctness risk this ADR exists to eliminate — mismatches fail softly, if at all.
* Bad, because response-shape knowledge stays implicit and scattered rather than centralized.

### Typed structs decoded at the boundary (chosen)

* Good, because it maximizes correctness and failure-visibility for a bounded, known-in-advance maintenance cost.
* Bad, because every new field or endpoint consumed requires a corresponding struct update.

### Schema/code-generation from the GraphQL schema

* Good, because it would keep generated types automatically in sync with the GraphQL schema, removing manual struct-maintenance drift.
* Bad, because it adds a build-time dependency and code-generation step, which is disproportionate tooling complexity for a single-developer CLI with a small, stable set of GraphQL queries.
* Neutral, because this remains worth revisiting if the GraphQL query surface grows substantially.

### Hybrid: typed core fields, raw passthrough for the rest

* Good, because it could reduce upfront struct-writing effort for rarely-used fields.
* Bad, because it reintroduces the exact correctness/failure-visibility gap this ADR exists to close, just for a subset of fields instead of all of them — and "which fields are which" becomes another thing to track.

## More Information

None beyond the above; this decision has no external-consumer-facing effect (unlike the `report.json` schema-envelope decision), since it concerns internal response handling rather than `ghlinks`'s own output shape.
