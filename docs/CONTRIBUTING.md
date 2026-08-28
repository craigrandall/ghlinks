# ghlinks Contributor Guide

## 1. Getting Started

**Clone and build:**

```bash
git clone <repo-url>
cd ghlinks
cargo build --release
```

The project is a deterministic collector for GitHub-hosted links:

> “A deterministic collector for a list of GitHub-hosted links (repo roots, files-in-a-repo, gists, GitHub Pages sites). It pulls structured facts from GitHub's API and checks one free, keyless API (Hacker News) for external mentions, and writes everything to one JSON file.”

## 2. Project Structure

- **`src/classify.rs`** – URL → `LinkKind` classification (repo root/file,
  gist, Pages site, user/org profile, unsupported GitHub URL, or unknown).
- **`src/github.rs`** – GitHub client, GraphQL-first: one round-trip pulls
  most repo facts, with a few small REST calls for what GraphQL doesn't
  expose well (languages, contributor count via the `Link` header, gists).
  Responses are decoded into typed structs at the boundary rather than
  traversed as raw `serde_json::Value` — see
  `ADRs/typed-response-models-over-raw-json-values.md` for why.
- **`src/discovery.rs`** – External mentions via Hacker News:
  > “This intentionally does NOT attempt sentiment or stance classification — it only collects raw signals (title, score, comment count).”
  Reddit discovery is deliberately not handled here — see
  `ADRs/reddit-mention-discovery-moves-to-synthesis-pass.md`.
- **`src/retry.rs`** – Shared retry/backoff decision logic used by both
  `github.rs` and `discovery.rs`. Deliberately dependency-free pure
  functions, decoupled from any HTTP I/O so they're unit-testable in
  isolation — see the module's own doc-comment and its test module for
  the pattern to follow if you add another retrying API call.
- **`src/model.rs`** – Data model and JSON schema (`LinkRecord`,
  `RepoData`, `GistData`, `ExternalDiscovery`, `RunSummary`, `Report`).
  The top-level `Report` is a schema-versioned envelope
  (`schema_version`, `run_summary`, `records`) rather than a bare array —
  see `ADRs/wrap-report-json-output-in-schema-versioned-envelope.md` for
  why.
- **`src/main.rs`** – CLI, orchestration, concurrency, and output.
  `run_batch()` is the extracted, directly-testable core of the
  concurrency/collection loop `main()` drives; `select_pages_candidate_index()`
  is the extracted, pure Pages candidate-selection policy.
- **`run.ps1`** – PowerShell wrapper for Windows usage.
- **`tests/e2e.rs`** – end-to-end integration test (T-1); see §6.

## 3. Development Workflow

1. **Pick a module** to work on (e.g., `classify`, `github`, `discovery`, `model`, `main`).
2. **Open an issue** describing the change (bugfix, feature, refactor).
3. **Create a branch**:
   ```bash
   git checkout -b feature/<short-name>
   ```
4. **Implement changes** with:
   - Clear separation of concerns.
   - Strong typing and explicit error handling.
   - No sentiment/stance classification in `discovery.rs`.

5. **Run tests**:
   ```bash
   cargo test
   ```
   This runs unit tests (in each module's own `#[cfg(test)]` block),
   HTTP-boundary and orchestration integration tests (which spin up a
   local `wiremock` mock server — see §6), and the end-to-end fixture test
   in `tests/e2e.rs` (which additionally spawns the compiled binary). No
   network access or GitHub token is required to run the full suite.

6. **Run a local end-to-end check**:
   ```bash
   ./run.ps1 -InputFile .\links.txt -OutputFile .\report.json
   ```

7. **Submit a pull request** with:
   - Summary of changes.
   - Rationale.
   - Notes on any new configuration or schema fields.

## 4. Coding Standards

- Use idiomatic Rust (`?` for error propagation, `Result<T, E>`, `Option<T>`).
- Prefer small, composable functions.
- Avoid `unsafe` unless absolutely necessary (and document it).
- Keep module boundaries clean:
  - `classify` should be pure and deterministic.
  - `github` should encapsulate all GitHub HTTP logic.
  - `discovery` should encapsulate external discovery logic.
  - `model` should remain schema-only.
  - `main` should orchestrate, not contain business logic.

## 5. Error Handling

- Use `anyhow::Context` for high-level context.
- Consider introducing domain-specific error enums (`GitHubError`, `DiscoveryError`) for finer-grained handling.
- Ensure partial failures are captured per link and surfaced in the output JSON.
- `fetch_errors` is currently `Vec<String>` (schema v2) — a deliberate
  decision to not restructure it into a typed/structured error object
  until the domain failure taxonomy itself is settled from evidence, not
  designed ahead of it. See `docs/failure-taxonomy-notes.md` for what the
  HTTP-boundary/orchestration tests (§6) have established so far about
  actual failure origins and behavior, and treat that as the prerequisite
  reading before proposing a schema v3 error model.

## 6. Testing

- **Unit tests**:
  - `classify.rs`: URL classification.
  - `github.rs`: parsing of GitHub responses (using fixtures).
  - `discovery.rs`: parsing of Hacker News responses.
  - `retry.rs`: retry/backoff decision logic — this module is the model
    example for testability, since it's pure functions with no HTTP I/O;
    prefer that shape (policy separated from mechanism) for new
    retry-adjacent logic rather than inlining decisions into the HTTP
    call sites.
  - `main.rs`: pure helper functions (`apply_releases`, `build_gist_data`,
    `describe_kind`, `select_pages_candidate_index` — the last one is the
    Pages candidate-selection *policy*, extracted specifically so it's
    unit-testable without live GitHub calls; the actual HTTP resolution
    loop in `process_link()` still owns the short-circuiting).

- **HTTP-boundary integration tests**
  (`src/github.rs::http_boundary_tests`, `src/discovery.rs::http_boundary_tests`):
  - Run the real `reqwest` client against a local `wiremock` mock server
    via `GitHub::with_base_url()` / `hacker_news()`'s `base_url` parameter,
    proving actual behavior at HTTP failure boundaries (status codes,
    retries, pagination, malformed bodies) — not just that a hand-built
    JSON string parses correctly.
  - `wiremock` is a **test-only** `[dev-dependencies]` entry — it is never
    linked into the release binary and is not required to run `ghlinks`
    itself.

- **Orchestration integration tests** (`src/main.rs::orchestration_tests`):
  - Exercise `process_link()`/`run_batch()` — the real functions `main()`
    calls — against mocked GitHub and Hacker News servers, covering
    per-link failure isolation, Pages candidate short-circuiting, HN
    success/zero-hits/failure paths (and that an HN failure never costs
    already-collected GitHub data), and the batch-level contract that one
    bad link never costs the others their records.

- **End-to-end integration test** (`tests/e2e.rs`):
  - Runs the actual compiled binary (via Cargo's
    `CARGO_BIN_EXE_ghlinks`) over `tests/fixtures/links.txt`, pointed at a
    mock GitHub server via the hidden `--github-base-url` flag (test-only;
    not advertised in `--help`, same pattern as the hidden `--token`
    flag), and validates the resulting `report.json`'s top-level contract.
  - This is deliberately the last test layer to rely on: it's a contract
    test of the complete pipeline built on top of the HTTP-boundary and
    orchestration layers above, not a substitute for either.

- **Property-based tests** (optional):
  - Fuzzing URL classification.

## 7. Documentation

`ghlinks` intentionally keeps most implementation detail in source
doc-comments rather than a separate design document — see each module's
top-level `//!` comment for its purpose, non-obvious decisions, and
failure behavior before writing new prose documentation elsewhere.

Some content is deliberately described in more than one place (e.g. module
responsibilities appear in both the README and here). If two documents
ever disagree, source code and its `//!` comments are authoritative for
*behavior*, ADRs are authoritative for *why* a decision was made, and
README/CONTRIBUTING are authoritative for the *user/contributor-facing
summary* — fix the summary to match the source, not the other way around.

- Update `README.md` when:
  - Adding new fields to the JSON schema.
  - Changing configuration or CLI behavior.
  - Introducing new modules or external sources.
  - Adding or changing a "Known limitation."

- Update the relevant module's `//!` doc-comment when:
  - Changing that module's externally observable behavior.
  - Adding a non-obvious design decision a future contributor would
    otherwise have to rediscover.

- Update the `.mmd` diagrams in `docs/` (see `docs/ARCHITECTURE.md` for
  what each one covers) when:
  - Introducing new modules or external sources.
  - Changing the sequencing of processing or data flow.
  - Changing the pipeline or concurrency model.

- Record a new ADR in `ADRs/` (see `ADRs/_README.md`) for any decision
  that's expensive to reverse, constrains later work, or had genuine
  alternatives worth recording the reasoning for — not for routine
  changes covered by the above.

## 8. AI-Assisted Review & Verification

Some contributions to this project — including drafts of source code,
ADRs, and documentation — are produced or reviewed with the help of an AI
assistant. That assistant frequently runs in a sandboxed environment with
**no Rust toolchain and no network access**, meaning any code it produces
or reviews has been read for correctness but has **not** been compiled,
linted, or executed. Treat AI-assisted output as unverified by
construction, regardless of how confident or polished it looks, until it
clears the same gate as any other change.

The actual verification gate for this project is local and manual. Before
considering any change — AI-assisted or not — ready for a pull request,
run all of the following, in order, and confirm each one is clean:

```bash
cargo fmt
cargo check
cargo clippy
cargo test
cargo build --release
```

A pull request that includes AI-assisted changes should say so, and
should confirm this sequence was actually run locally rather than
inferred from the code "looking right." If any step above hasn't been
run, say that explicitly in the PR rather than implying it has — this
project would rather show an honest "not yet verified" than a false
green.
