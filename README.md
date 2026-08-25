# ghlinks

A deterministic collector for GitHub‑hosted links. Given a list of URLs, `ghlinks` classifies each link, retrieves structured metadata from GitHub’s API, optionally discovers external mentions via a free, keyless API (Hacker News), and produces a single JSON report describing all collected facts.

The tool intentionally avoids sentiment or stance classification. It only collects raw signals.

This README describes the architecture, configuration, error model, JSON schema, usage, and future roadmap.

Additional project documentation may be found in the `docs` folder (e.g. architecture, design, contributing).

---

## Purpose

`ghlinks` provides a reproducible way to analyze GitHub content at scale. It is designed for workflows that require:

- Deterministic metadata collection  
- Structured output suitable for downstream processing  
- Repeatable runs over evolving link lists  
- Optional external discovery via free APIs  

It supports:

- GitHub repositories  
- GitHub gists  
- GitHub Pages sites  
- Files within repositories  

---

## Architecture Overview

The system is composed of five modules:

### 1. `classify`

Parses raw URLs and maps them to a structured `LinkKind`:

- Repository root  
- File within a repository  
- Gist  
- GitHub Pages site  

This module determines which downstream collectors should run.

### 2. `github`

Responsible for all authenticated GitHub API interactions:

- Repository metadata  
- License information  
- Releases and tags  
- Languages  
- Issues and pull requests  
- Contributors  

Uses `reqwest` and respects a maximum response size limit.

### 3. `discovery`

Queries a free, unauthenticated API (Hacker News) to identify external mentions. It collects:

- Title  
- Score  
- Comment count  

It does not attempt sentiment or stance classification.

### 4. `model`

Defines the structured output schema. All collected data is serialized using `serde`.

### 5. `main`

Orchestrates the pipeline:

- Reads input file  
- Classifies links  
- Executes GitHub and discovery collectors concurrently  
- Aggregates results  
- Writes JSON output  

Concurrency is implemented using `futures::stream`.

---

## Concurrency Model

`ghlinks` processes links concurrently to improve throughput. Concurrency is bounded to avoid overwhelming GitHub’s API or external services.

Key points:

- Concurrency is configurable via CLI.  
- All tasks are independent; failures are isolated per link.  
- Rate limits are respected; transient failures are retried conservatively.  

---

## Error Model

Errors fall into three categories:

### 1. Network errors  

Connection failures, timeouts, or unreachable endpoints.

### 2. API errors  

GitHub responses such as:

- 404 (not found)  
- 403 (rate limited)  
- 500 (server error)  

### 3. Parsing errors  

Invalid JSON, unexpected schema changes, or truncated responses.

All errors are captured per link and included in the output JSON under an `errors` field. The tool continues processing other links even when individual failures occur.

---

## JSON Schema

The output JSON contains three top‑level sections:

```json
{
  "run_metadata": { ... },
  "repos": [ ... ],
  "gists": [ ... ]
}
```

### `run_metadata`

- Timestamp  
- Input file path  
- Whether external discovery was enabled  

### `repos[]`

Each entry includes:

- URL  
- Owner  
- Repository name  
- Description  
- Topics  
- License (SPDX ID and name)  
- Default branch  
- Creation and update timestamps  
- Releases  
- Latest release  
- Release count in last 12 months  
- Languages (byte counts)  
- Primary language  
- Community metrics  
- External mentions (if enabled)  
- Errors  

### `gists[]`

Each entry includes:

- URL  
- Owner  
- Gist ID  
- Description  
- Creation and update timestamps  
- Files and detected languages  
- Errors  

A complete example is available in `verify-v0.12-research-quality-baseline-report.json`.

---

## Configuration

Configuration is provided via CLI flags and environment variables.

### GitHub Token

A GitHub personal access token is required for authenticated API calls. It must be supplied via environment variable:

```
$env:GITHUB_TOKEN = "<token>"
```

### CLI Flags

```
ghlinks --input links.txt --output report.json
```

Optional flags:

- `--skip-external`  
- `--concurrency <n>`  
- `--max-response-bytes <n>`  

---

## Usage

### Basic usage

```
./run.ps1 -InputFile .\links.txt -OutputFile .\report.json
```

### Input format

A plain text file containing one URL per line.

### Output format

A single JSON file containing structured metadata for all links.

---

## Contributing

Contributions are welcome. Recommended guidelines:

- Add unit tests for URL classification and GitHub response parsing.  
- Add integration tests for end‑to‑end runs.  
- Follow Rust idioms for error handling (`thiserror`, `anyhow::Context`).  
- Keep module boundaries clean and cohesive.  
- Document new fields in both `model.rs` and this README.  

See `docs\CONTRIBUTING.md` for more details.

---

## Roadmap

Planned enhancements:

- Markdown report generation  
- Additional external discovery sources  
- Structured error types  
- Configurable retry and backoff policies  
- Optional local caching of GitHub responses  
- Dependency extraction from repository manifests  

---

## License

Dual-licensed under MIT or Apache-2.0, at your option. See
`LICENSE-MIT` and `LICENSE-APACHE`.
