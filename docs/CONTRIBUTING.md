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

- **`src/classify.rs`** – URL → `LinkKind` classification.
- **`src/github.rs`** – GitHub API client and metadata collection.
- **`src/discovery.rs`** – External mentions via Hacker News:
  > “This intentionally does NOT attempt sentiment or stance classification — it only collects raw signals (title, score, comment count).”
- **`src/model.rs`** – Data model and JSON schema.
- **`src/main.rs`** – CLI, orchestration, concurrency, and output.
- **`run.ps1`** – PowerShell wrapper for Windows usage.

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

5. **Run tests** (once added):
   ```bash
   cargo test
   ```

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

## 6. Testing

- **Unit tests**:
  - `classify.rs`: URL classification.
  - `github.rs`: parsing of GitHub responses (using fixtures).
  - `discovery.rs`: parsing of Hacker News responses.

- **Integration tests**:
  - End-to-end run over a small fixture input file.
  - JSON schema validation.

- **Property-based tests** (optional):
  - Fuzzing URL classification.

## 7. Documentation

- Update `README.md` and `DESIGN.md` when:
  - Adding new fields to the JSON schema.
  - Changing configuration or CLI behavior.
  - Introducing new modules or external sources.

- Update `ARCHITECTURE.md` when:
  - Introducing new modules or external sources.
  - Changing the sequencing of processing or data flow.
  - Changing the pipeline.
