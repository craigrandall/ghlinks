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
    fn canonicalize_returns_none_only_for_unparseable_urls() {
        assert_eq!(canonicalize("not a url"), None);
        // Recognized-but-out-of-scope still canonicalizes.
        assert_eq!(
            canonicalize("https://github.com/eugeneyan/"),
            Some("https://github.com/eugeneyan".to_string())
        );
    }
}