//! T-1: end-to-end integration test.
//!
//! Invokes the *actual compiled `ghlinks` binary* (via
//! `CARGO_BIN_EXE_ghlinks`, Cargo's standard mechanism for this — see
//! https://doc.rust-lang.org/cargo/reference/environment-variables.html)
//! over a small deterministic fixture file, with `--github-base-url`
//! pointed at a local `wiremock` server so the run is fully offline and
//! reproducible, and `--skip-external` so Hacker News is never called —
//! HN discovery has its own dedicated success/failure/isolation coverage
//! in `src/main.rs::orchestration_tests` (via `--hn-base-url`'s
//! equivalent, `hn_base_url`), so this test doesn't need to duplicate
//! that; it's here to prove the complete pipeline runs end-to-end, not to
//! re-prove either API integration individually.
//!
//! This is deliberately the *last* layer added (per the agreed testing
//! sequence: T-3 -> T-7 -> T-4 -> T-1) — it's a contract test of the
//! complete system built on top of already-established HTTP-boundary
//! (T-3) and orchestration (T-4) behavior, not a substitute for either.
//!
//! Existing unit/HTTP-boundary/orchestration tests cannot establish that
//! the complete pipeline — CLI parsing, file I/O, and JSON serialization
//! included — actually works together; this is the one test that does.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn unique_temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let unique = format!(
        "ghlinks-e2e-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    p.push(unique);
    p
}

// `multi_thread`, not the default `current_thread`: this test blocks a
// worker thread synchronously waiting on the spawned child process (see
// `Command::status()` below), and the mock server's own request-handling
// tasks run on this same Tokio runtime. On a single-threaded runtime, that
// blocking wait would starve the mock server of the chance to ever answer
// the child process's HTTP requests, deadlocking the test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn end_to_end_run_over_a_fixture_produces_a_structurally_valid_report() {
    // --- Arrange: a mock GitHub API standing in for api.github.com ---
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/graphql"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "repository": {
                        "description": "e2e fixture repo",
                        "stargazerCount": 1,
                        "releases": { "totalCount": 0 }
                    }
                }
            })),
        )
        .mount(&server)
        .await;
    // languages / contributors — anything else GET
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/links.txt");
    let output_path = unique_temp_path("report.json");

    // --- Act: run the real binary ---
    let bin = env!("CARGO_BIN_EXE_ghlinks");
    let status = Command::new(bin)
        .arg("--input")
        .arg(&fixture)
        .arg("--output")
        .arg(&output_path)
        .arg("--github-base-url")
        .arg(server.uri())
        .arg("--skip-external")
        .arg("--concurrency")
        .arg("1")
        .arg("--delay-ms")
        .arg("0")
        .arg("--max-retries")
        .arg("1")
        .arg("--timeout-secs")
        .arg("10")
        .env("GITHUB_TOKEN", "e2e-test-token-not-real")
        .status()
        .expect("failed to spawn the ghlinks binary");

    assert!(
        status.success(),
        "ghlinks exited with a failure status: {status:?}"
    );

    // --- Assert: the top-level report.json contract ---
    let raw = fs::read_to_string(&output_path).expect("report.json was not written");
    let report: Value = serde_json::from_str(&raw).expect("report.json was not valid JSON");
    let _ = fs::remove_file(&output_path);

    let obj = report
        .as_object()
        .expect("top level must be a JSON object, not an array");
    assert_eq!(
        obj.keys()
            .cloned()
            .collect::<std::collections::BTreeSet<String>>(),
        ["schema_version", "run_summary", "records"]
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::BTreeSet<String>>()
    );
    assert_eq!(report["schema_version"], 2);

    let records = report["records"]
        .as_array()
        .expect("records must be an array");
    assert_eq!(records.len(), 2, "one record per fixture line");

    let repo_record = records
        .iter()
        .find(|r| r["link_kind"] == "repo_root")
        .expect("expected a repo_root record for the valid fixture URL");
    assert!(
        repo_record["fetch_errors"].as_array().unwrap().is_empty(),
        "the valid repo URL should have collected cleanly: {repo_record:#}"
    );
    assert_eq!(
        repo_record["repo_data"]["description"], "e2e fixture repo",
        "collected data should actually reach the final JSON"
    );

    let unknown_record = records
        .iter()
        .find(|r| r["link_kind"] == "unknown")
        .expect("expected an unknown record for the unparseable fixture line");
    assert!(
        !unknown_record["fetch_errors"]
            .as_array()
            .unwrap()
            .is_empty(),
        "an unparseable URL should be recorded as an error, not silently dropped"
    );

    // Per-link failure isolation, proven end-to-end: the bad line did not
    // prevent the good line from producing a clean record, and vice versa.
    assert_eq!(report["run_summary"]["records_with_errors"], 1);
    assert_eq!(report["run_summary"]["total_urls"], 2);
}
