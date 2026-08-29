//! Turns a raw URL string into a structured `LinkKind` so downstream code
//! knows which GitHub API calls are relevant.
//!
//! Normalization applied here is deliberate, not incidental: host is
//! lowercased, a trailing `.git` on the repo segment is trimmed, empty
//! path segments (e.g. a trailing slash) are dropped, and query strings /
//! URL fragments are ignored entirely, since none of those change which
//! GitHub resource a URL identifies. `canonicalize()` below produces a
//! normalized string form reflecting exactly these same rules, recorded
//! alongside the original input URL for provenance.

#[derive(Debug, Clone, serde::Serialize)]
pub enum LinkKind {
    /// e.g. https://github.com/{owner}/{repo}
    RepoRoot {
        owner: String,
        repo: String,
    },
    /// e.g. https://github.com/{owner}/{repo}/blob/{branch}/{path...}
    /// Note: despite the field name, `{branch}` here is whatever ref the
    /// URL used — a branch, tag, or full commit SHA are all valid and
    /// indistinguishable from the URL shape alone.
    RepoFile {
        owner: String,
        repo: String,
        branch: String,
        path: String,
    },
    /// e.g. https://gist.github.com/{owner}/{gist_id}
    Gist {
        owner: String,
        gist_id: String,
    },
    /// e.g. https://{owner}.github.io/{path...} — GitHub Pages, not
    /// guaranteed to map 1:1 to a repo of the same name, so we record
    /// candidate repos to check rather than assuming.
    PagesSite {
        owner: String,
        path: String,
        candidates: Vec<(String, String)>,
    },
    /// e.g. https://github.com/{login} — a user or organization profile
    /// page, not a repository. Recognized and deliberately out of scope,
    /// which is different from a URL we simply don't understand at all
    /// (see `Unknown`).
    UserOrOrgProfile {
        login: String,
    },
    /// A GitHub URL we recognize but do not collect as a repository link
    /// (e.g. .../issues, .../tree/branch/subdir).
    UnsupportedGithubUrl,
    /// A URL we could not parse, or whose host we don't recognize at all.
    Unknown,
}

pub fn classify(raw_url: &str) -> LinkKind {
    let Ok(u) = url::Url::parse(raw_url) else {
        return LinkKind::Unknown;
    };
    let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
    let segs: Vec<String> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();

    if host == "gist.github.com" {
        if segs.len() >= 2 {
            return LinkKind::Gist {
                owner: segs[0].clone(),
                gist_id: segs[1].clone(),
            };
        }
        return LinkKind::Unknown;
    }

    if host == "github.com" || host == "www.github.com" {
        if segs.len() == 1 {
            return LinkKind::UserOrOrgProfile {
                login: segs[0].clone(),
            };
        }
        if segs.len() >= 2 {
            let owner = segs[0].clone();
            let repo = segs[1].trim_end_matches(".git").to_string();
            if repo.is_empty() {
                return LinkKind::Unknown;
            }
            // .../blob/{branch}/{path...}
            if segs.len() >= 5 && segs[2] == "blob" {
                let branch = segs[3].clone();
                let path = segs[4..].join("/");
                return LinkKind::RepoFile {
                    owner,
                    repo,
                    branch,
                    path,
                };
            }
            if segs.len() == 2 {
                return LinkKind::RepoRoot { owner, repo };
            }
            return LinkKind::UnsupportedGithubUrl;
        }
        return LinkKind::Unknown;
    }

    if host.ends_with(".github.io") {
        let owner = host.trim_end_matches(".github.io").to_string();
        let path = segs.join("/");
        // Two common patterns: the Pages content lives in the
        // `{owner}.github.io` repo itself, OR in a project repo named after
        // the first path segment (deployed via a gh-pages branch / docs
        // folder). We record both as candidates; the caller checks which
        // (if either) actually exists rather than guessing.
        let mut candidates = vec![(owner.clone(), format!("{owner}.github.io"))];
        if let Some(first) = segs.first() {
            candidates.push((owner.clone(), first.clone()));
        }
        return LinkKind::PagesSite {
            owner,
            path,
            candidates,
        };
    }

    LinkKind::Unknown
}

/// Normalized string form of a URL, applying the same rules `classify`
/// uses internally: lowercased scheme+host, a trailing `.git` and
/// trailing slash trimmed, query string and fragment dropped. Returns
/// `None` only when the URL can't be parsed at all — a
/// recognized-but-out-of-scope URL (e.g. a user profile) still gets a
/// canonical form here even though `classify` won't collect it.
pub fn canonicalize(raw_url: &str) -> Option<String> {
    let u = url::Url::parse(raw_url).ok()?;
    let scheme = u.scheme();
    let host = u.host_str()?.to_ascii_lowercase();
    let mut segs: Vec<String> = u
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(|p| p.to_string()).collect())
        .unwrap_or_default();
    if (host == "github.com" || host == "www.github.com") && segs.len() >= 2 {
        segs[1] = segs[1].trim_end_matches(".git").to_string();
    }
    let path = segs.join("/");
    Some(if path.is_empty() {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}/{path}")
    })
}

#[cfg(test)]
mod tests {
    use super::{canonicalize, classify, LinkKind};

    #[test]
    fn normalizes_common_repository_root_forms() {
        for url in [
            "https://github.com/Owner/Repo/",
            "http://www.github.com/Owner/Repo.git",
        ] {
            assert!(matches!(classify(url), LinkKind::RepoRoot { .. }));
        }
    }

    #[test]
    fn does_not_misclassify_repository_subpages_as_roots() {
        assert!(matches!(
            classify("https://github.com/owner/repo/issues"),
            LinkKind::UnsupportedGithubUrl
        ));
    }

    #[test]
    fn identifies_files_gists_and_pages() {
        assert!(matches!(
            classify("https://github.com/o/r/blob/main/src/lib.rs"),
            LinkKind::RepoFile { .. }
        ));
        assert!(matches!(
            classify("https://gist.github.com/o/abc123"),
            LinkKind::Gist { .. }
        ));
        assert!(matches!(
            classify("https://o.github.io/project"),
            LinkKind::PagesSite { .. }
        ));
    }

    #[test]
    fn distinguishes_user_org_profile_from_genuinely_unknown() {
        assert!(matches!(
            classify("https://github.com/eugeneyan/"),
            LinkKind::UserOrOrgProfile { login } if login == "eugeneyan"
        ));
        assert!(matches!(
            classify("https://github.com/modelcontextprotocol"),
            LinkKind::UserOrOrgProfile { login } if login == "modelcontextprotocol"
        ));
        // A host we don't recognize at all is still genuinely Unknown.
        assert!(matches!(
            classify("https://github.blog/some/post/"),
            LinkKind::Unknown
        ));
    }

    #[test]
    fn canonicalize_strips_query_fragment_git_suffix_and_trailing_slash() {
        assert_eq!(
            canonicalize("HTTPS://GitHub.com/Owner/Repo.git/?tab=readme#section"),
            Some("https://github.com/Owner/Repo".to_string())
        );
        assert_eq!(
            canonicalize("https://github.com/o/r/"),
            Some("https://github.com/o/r".to_string())
        );
    }

    #[test]
    fn classify_ignores_query_string_and_fragment_on_repo_root_urls() {
        // canonicalize() already had a test for this; classify() itself
        // (the function that decides *what kind* of link this is) hadn't
        // been separately checked against a query/fragment-bearing URL.
        assert!(matches!(
            classify("https://github.com/owner/repo?tab=readme#readme"),
            LinkKind::RepoRoot { .. }
        ));
    }

    #[test]
    fn trailing_dot_git_is_trimmed_even_when_repeated() {
        // trim_end_matches(".git") strips every trailing occurrence, not
        // just one — "repo.git.git" becomes "repo", not "repo.git". This
        // documents that actual (somewhat surprising) behavior so a future
        // change to it is a deliberate decision, not an accidental
        // regression.
        assert!(matches!(
            classify("https://github.com/owner/repo.git.git"),
            LinkKind::RepoRoot { repo, .. } if repo == "repo"
        ));
    }

    #[test]
    fn dot_git_substring_in_the_middle_of_a_repo_name_is_left_alone() {
        // Only a *trailing* ".git" is special-cased; one that isn't at the
        // end (e.g. a repo literally named "my.github.repo") must survive
        // untouched.
        assert!(matches!(
            classify("https://github.com/owner/my.github.repo"),
            LinkKind::RepoRoot { repo, .. } if repo == "my.github.repo"
        ));
    }

    #[test]
    fn github_enterprise_style_custom_domains_are_unknown_not_repo_root() {
        // classify() only recognizes github.com/www.github.com and
        // *.github.io. A GitHub Enterprise Server instance on its own
        // domain (e.g. github.mycompany.com) is not special-cased and
        // falls through to Unknown. This documents that as current,
        // intentional scope — ghlinks targets github.com only — rather
        // than leaving it as an unstated assumption. If GHE support is
        // ever wanted, that's a scope decision (arguably ADR-worthy, since
        // it'd need a configurable host allowlist), not a bug fix.
        assert!(matches!(
            classify("https://github.mycompany.com/owner/repo"),
            LinkKind::Unknown
        ));
    }

    #[test]
    fn pages_candidates_can_duplicate_when_the_first_path_segment_matches_the_owner_repo() {
        // owner.github.io/owner.github.io is a degenerate but legal URL:
        // both candidate-generation rules produce the same (owner, repo)
        // pair, so `candidates` ends up with a literal duplicate. It's
        // harmless (main.rs's resolution loop just checks the same repo
        // twice and stops at the first success either way) but wasteful —
        // one avoidable extra GitHub API call per occurrence. Documenting
        // the current behavior here rather than silently dedupe-ing it,
        // since that's a small, separate fix main.rs's Pages-resolution
        // loop or classify()'s candidate generation could make.
        if let LinkKind::PagesSite { candidates, .. } =
            classify("https://owner.github.io/owner.github.io")
        {
            assert_eq!(
                candidates,
                vec![
                    ("owner".to_string(), "owner.github.io".to_string()),
                    ("owner".to_string(), "owner.github.io".to_string()),
                ]
            );
        } else {
            panic!("expected a PagesSite classification");
        }
    }

    #[test]
    fn blob_ref_names_containing_a_slash_are_misparsed_into_kind_repo_file() {
        // KNOWN LIMITATION, documented rather than silently fixed: a
        // branch/tag name containing a slash (e.g. "feature/foo", a
        // completely legal Git ref name) is indistinguishable from a
        // deeper path from the URL shape alone. classify() takes the
        // segment immediately after "blob" as the whole ref, so
        // .../blob/feature/foo/src/lib.rs is parsed as ref "feature" with
        // path "foo/src/lib.rs" — silently wrong, not an error. GitHub's
        // own web UI resolves this ambiguity by checking which ref
        // actually exists in the repo, which classify() cannot do (it's
        // deliberately pure/offline, with no API access). This is worth a
        // README "Known limitations" entry; it is NOT fixed here, since
        // fixing it would require classify() to make an API call, which
        // is out of scope for this module by design.
        let kind = classify("https://github.com/owner/repo/blob/feature/foo/src/lib.rs");
        assert!(matches!(
            kind,
            LinkKind::RepoFile { ref branch, ref path, .. }
                if branch == "feature" && path == "foo/src/lib.rs"
        ));
    }

    #[test]
    fn percent_encoded_path_segments_are_preserved_not_decoded() {
        // Documents current behavior rather than asserting it's ideal:
        // `url::Url::path_segments()` yields segments still
        // percent-encoded, and nothing in classify()/canonicalize()
        // decodes them. A file path containing a space (encoded as %20 in
        // the URL) comes through literally as "%20", not " ". Whether
        // that should be decoded is a separate product decision — this
        // test just locks in what actually happens today so a future
        // change to it is deliberate.
        let kind = classify("https://github.com/owner/repo/blob/main/a%20file.rs");
        assert!(matches!(
            kind,
            LinkKind::RepoFile { ref path, .. } if path == "a%20file.rs"
        ));
    }
}