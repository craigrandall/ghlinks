---
status: "accepted"
date: 2026-08-27
decision-makers: Craig
consulted: Claude (Anthropic), ChatGPT — see the two attached progress/review conversations that preceded this decision
informed: n/a — single-developer project
---

# Use `wiremock` as a test-only HTTP-mocking dependency for HTTP-boundary integration tests (T-3/T-4/T-1)

## Context and Problem Statement

`ghlinks`'s value proposition is "honest failure reporting" at the GitHub/HN
API boundary — but prior to this decision, every test of `github.rs`
proved only that hand-built JSON strings deserialize correctly. Nothing
proved that the *actual* `reqwest` HTTP client, given real HTTP status
codes, headers, retries, and pagination, behaves the way the retry/backoff
policy (`retry.rs`) and typed-response models (`github.rs`) assume it does.
`GitHub::with_base_url()` already existed specifically as a hook for this,
per its own doc-comment, but nothing used it.

The question: **how should HTTP-boundary tests actually reach a real HTTP
client — hand-roll a minimal mock server over `tokio::net::TcpListener`
using only crates already in the dependency tree, or add a purpose-built
HTTP-mocking crate as a test-only dependency?**

## Decision Drivers

* `ghlinks`'s core value depends on external HTTP APIs; the missing tests
  are precisely at those boundaries (T-3), and untested boundaries are
  where "honest failure reporting" is most likely to be silently wrong.
* Keep the **production** dependency footprint minimal (an existing,
  explicit project value) — this is not the same principle as "never add
  test dependencies."
* Avoid building and maintaining a bespoke HTTP-mocking framework in order
  to test an HTTP client — that inverts the goal (the tests exist to prove
  `ghlinks`, not to create and then separately validate new test
  infrastructure).
* `github.rs::with_base_url()` already exists as a ready seam for exactly
  this; the chosen approach should use it as-is, not require a preceding
  production-code refactor.
* Parallel test execution (`cargo test` runs tests concurrently by
  default) needs test-server isolation that doesn't leak state or ports
  across tests.

## Considered Options

* Hand-rolled mock server over `tokio::net::TcpListener`, using only
  crates already in `Cargo.toml`
* Add `wiremock` as a `[dev-dependencies]` entry (chosen)

## Decision Outcome

Chosen option: **add `wiremock` as a `[dev-dependencies]` entry**, because
it is purpose-built for exactly this (black-box HTTP-boundary testing of a
Rust HTTP client), it exercises the real `reqwest` request path through
the existing `with_base_url()` seam without any production-code changes,
and it avoids trading "missing HTTP-boundary tests" for "a new,
self-built HTTP mock server that itself needs to be trusted and
maintained."

### Consequences

* Good, because T-3/T-4/T-1 tests now exercise the real `reqwest`
  request → typed-response path against controlled HTTP responses
  (status codes, headers, sequenced retries, multi-page pagination)
  instead of only proving that hand-built JSON strings parse.
* Good, because `wiremock` supports per-test request-count assertions
  (e.g. "the second Pages candidate was never queried," "exactly
  `max_retries` attempts were made") — proving requests were or weren't
  made, not just what a given response produces.
* Good, because each test's `MockServer::start()` binds an isolated
  server on a random local port, which fits Rust's default parallel test
  execution without cross-test interference.
* Good, because it does **not** become a production/runtime dependency —
  `[dev-dependencies]` entries are never linked into `cargo build
  --release`.
* Bad, because it grows the project's development/test dependency graph
  (transitively includes `hyper`, `hyper-util`, `http`,
  `http-body-util`, and others as of `wiremock` 0.6.x) — a real cost for
  a project that has otherwise kept its dependency set deliberately
  small.
* Neutral, because this cost is paid once, at test-build time only, and
  never appears in `cargo build --release`'s dependency graph.

### Confirmation

Implementation is compliant when:
- `wiremock` appears under `[dev-dependencies]` in `Cargo.toml`, never
  under `[dependencies]`.
- `cargo build --release` succeeds and its dependency graph does not
  include `wiremock` or any dependency introduced solely by it.
- HTTP-boundary tests (`src/github.rs::http_boundary_tests`) and
  orchestration tests (`src/main.rs::orchestration_tests`) exercise
  `GitHub::with_base_url()` pointed at a local `wiremock` server, not a
  hand-rolled server.
- `docs/CONTRIBUTING.md` documents `wiremock` explicitly as test
  infrastructure, not an application dependency.

## Pros and Cons of the Options

### Hand-rolled mock server over `tokio::net::TcpListener`

* Good, because it adds no new dependency.
* Good, because it keeps the project's dependency set maximally lean.
* Bad, because accepting connections, parsing enough of raw HTTP to
  identify method/path/body, returning correctly formed responses
  (status, headers, body), supporting sequenced multi-response scenarios
  (retry, pagination), and clean shutdown is, in aggregate, writing a
  small HTTP mocking framework — test infrastructure that itself has no
  test coverage guarantee.
* Bad, because that infrastructure is more code to review and maintain
  than the tests it exists to support.

### Add `wiremock` as a `[dev-dependencies]` entry

* Good, because it is purpose-built for black-box HTTP-client testing:
  request matching, predetermined/sequenced responses, and
  invocation-count expectations, out of the box.
* Good, because its mock servers run on isolated random ports, matching
  Rust's parallel test execution model without extra coordination code.
* Neutral, because it introduces a nontrivial transitive dependency tree,
  but strictly as a dev/test-time cost.
* Bad, because it is one more crate's release cadence and security
  surface to track, even if only at test-build time.

## More Information

This decision was reached through extended deliberation captured in two
prior conversations attached alongside this project's review documents
(`2026-08-27 Review outstanding docs, test, and features work in
'ghlinks'`, prompts #2–#4) — this ADR formalizes that already-reasoned
outcome in the project's standard MADR format rather than re-litigating
it. See `docs/CONTRIBUTING.md` §6 for how each test layer (unit,
HTTP-boundary, orchestration, end-to-end) uses this dependency in
practice.
