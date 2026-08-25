//! Turns a raw URL string into a structured `LinkKind` so downstream code
//! knows which GitHub API calls are relevant.

#[derive(Debug, Clone, serde::Serialize)]
pub enum LinkKind {
    /// e.g. https://github.com/{owner}/{repo}
    RepoRoot {
        owner: String,
        repo: String,
    },
    /// e.g. https://github.com/{owner}/{repo}/blob/{branch}/{path...}
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
    /// A GitHub URL we recognize but do not collect as a repository link.
    UnsupportedGithubUrl,
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

#[cfg(test)]
mod tests {
    use super::{classify, LinkKind};

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
}
