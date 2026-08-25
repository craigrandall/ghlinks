# ghlinks Architectural and Implementation Design

This document provides a detailed technical walkthrough of the architecture, data flow, module responsibilities, concurrency model, error handling strategy, and implementation notes for the `ghlinks` project.

It is intended for contributors, maintainers, and anyone extending or integrating the system.

---

## 1. Design Goals

The design of `ghlinks` is driven by four core goals:

1. **Determinism**  
   Runs should produce consistent results given the same inputs, subject only to changes in upstream GitHub or external APIs.

2. **Modularity**  
   Each stage of the pipeline is isolated, testable, and replaceable.

3. **Structured Output**  
   All collected data is represented using strongly typed Rust structs and serialized via `serde`.

4. **Minimal External Dependencies**  
   Only one free, keyless API (Hacker News) is used for external discovery. No sentiment analysis or stance classification is performed.

The README states this explicitly:  
> “It does **not** attempt sentiment or stance classification — it only collects raw signals (title, score, comment count).”

---

## 2. High-Level Architecture

The system is composed of five modules:

```
src/
  classify.rs
  discovery.rs
  github.rs
  model.rs
  main.rs
```

Each module has a single responsibility. The pipeline is orchestrated by `main.rs`.

See `ARCHITECTURE.md` for more details (e.g. diagrams).

---

## 3. Module Responsibilities

### 3.1 classify.rs

Purpose: Convert raw URLs into structured `LinkKind` variants.

The module’s header states:  
> “Turns a raw URL string into a structured `LinkKind` so downstream code knows which GitHub API calls are relevant.”

Responsibilities:
- Parse GitHub repo URLs
- Parse GitHub gist URLs
- Parse GitHub Pages URLs
- Normalize owner/repo identifiers
- Provide structured classification for downstream collectors

Design notes:
- This module should be pure and deterministic.
- It is a prime candidate for unit tests.

---

### 3.2 github.rs

Purpose: Interact with GitHub’s REST API.

The module begins with:

> `use anyhow::{Context, Result};`  
> `use reqwest::Client;`

Responsibilities:
- Fetch repository metadata
- Fetch license information
- Fetch releases and tags
- Fetch languages
- Fetch contributors
- Enforce maximum response size (`MAX_RESPONSE_BYTES`)

Design notes:
- All GitHub calls should be wrapped in small, composable async functions.
- Errors should be contextualized using `anyhow::Context`.
- Rate limiting and pagination must be handled carefully.

---

### 3.3 discovery.rs

Purpose: Discover external mentions using free, unauthenticated APIs.

The module states:

> “This intentionally does NOT attempt sentiment or stance classification — it only collects raw signals (title, score, comment count).”

Responsibilities:
- Query Hacker News API
- Extract raw metadata (title, score, comments)
- Associate mentions with GitHub repos

Design notes:
- External discovery is optional and should be controlled via CLI.
- No stance or sentiment classification is performed.

---

### 3.4 model.rs

Purpose: Define the structured output schema.

The module contains:

> `#[derive(Serialize, Default, Debug)]`

Responsibilities:
- Represent all collected data using Rust structs
- Provide a stable schema for JSON serialization
- Support future extensibility (e.g., additional external sources)

Design notes:
- All fields should be documented.
- Optional fields should use `Option<T>` consistently.

---

### 3.5 main.rs

Purpose: Orchestrate the entire pipeline.

The module imports:

> `use clap::Parser;`  
> `use futures::stream::{self, StreamExt};`

Responsibilities:
- Parse CLI arguments
- Load input file
- Classify URLs
- Execute GitHub and discovery collectors concurrently
- Aggregate results
- Serialize final JSON output

Design notes:
- Concurrency should be bounded and configurable.
- Partial failures should be captured per link, not globally.

---

## 4. Data Flow

See `ARCHITECTURE.md` for more a sequence diagrams.

---

## 5. Concurrency Model

Concurrency is implemented using `futures::stream::StreamExt` with bounded parallelism.

Key properties:
- Each link is processed independently.
- Failures do not affect other tasks.
- Concurrency level is configurable.
- GitHub API rate limits must be respected.

Design considerations:
- Use `buffer_unordered(n)` where `n` is user-configurable.
- Consider exponential backoff for transient failures.
- Ensure that large responses do not exceed `MAX_RESPONSE_BYTES`.

---

## 6. Error Handling Strategy

Errors are captured per link and included in the output JSON.

### Error categories:

1. Network errors  
2. GitHub API errors  
3. Parsing errors  
4. External API errors  

### Design principles:

- Use `anyhow::Context` to annotate errors.
- Do not abort the entire run unless the input file is unreadable.
- Include an `errors: Vec<String>` field in each output struct.

---

## 7. JSON Schema

The schema is defined in `model.rs`. It includes:

- `RepoData`
- `ReleaseEntry`
- `ExternalMention`
- `GistData`
- `RunMetadata`

Design notes:
- All structs derive `Serialize`.
- Optional fields use `Option<T>`.
- Collections use `Vec<T>` or `BTreeMap<String, T>`.

---

## 8. Implementation Notes

### 8.1 Determinism

- All GitHub calls use fixed API endpoints.
- No randomization or nondeterministic ordering.
- External discovery is limited to a single free API.

### 8.2 Response Size Limits

`github.rs` defines:

> `const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;`

This prevents runaway memory usage.

### 8.3 CLI Integration

`main.rs` uses `clap::Parser` for robust argument parsing.

### 8.4 Serialization

All output is serialized using `serde_json`.

### 8.5 PowerShell Integration

The repository includes `run.ps1` for Windows users.

---

## 9. Testing Strategy

Recommended tests:

### Unit tests

- URL classification (`classify.rs`)
- GitHub response parsing (`github.rs`)
- External mention parsing (`discovery.rs`)

### Integration tests

- End-to-end run over a small fixture input file
- JSON schema validation

### Property-based tests

- URL classification fuzzing

---

## 10. Future Extensions

Potential enhancements:

- Markdown report generation
- Additional external discovery sources
- Structured error types using `thiserror`
- Local caching of GitHub responses
- Dependency extraction from repository manifests
- Configurable retry/backoff policies
- Parallel output writers (JSON + Markdown)

---

## 11. Summary

`ghlinks` is a deterministic, modular, strongly typed Rust system for collecting structured metadata about GitHub-hosted links. Its architecture is intentionally simple, extensible, and robust against partial failures. This design document provides the deeper context needed to maintain, extend, and integrate the system effectively.
