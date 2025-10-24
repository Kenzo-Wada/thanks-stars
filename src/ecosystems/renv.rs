use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use url::Url;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum RenvDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Default)]
pub struct RenvDiscoverer;

impl RenvDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, RenvDiscoveryError> {
        let path = project_root.join("renv.lock");
        let contents = fs::read_to_string(&path).map_err(|source| RenvDiscoveryError::Io {
            path: path.display().to_string(),
            source,
        })?;

        let lock: RenvLock =
            serde_json::from_str(&contents).map_err(|source| RenvDiscoveryError::Json {
                path: path.display().to_string(),
                source,
            })?;

        let mut seen = BTreeSet::new();
        let mut repositories = Vec::new();

        for package in lock.packages.values() {
            if let Some((owner, name)) = package.github_owner_repo() {
                if seen.insert((owner.clone(), name.clone())) {
                    let url = format!("https://github.com/{owner}/{name}");
                    if let Some(mut repository) = parse_github_repository(&url) {
                        repository.via = Some("renv.lock".to_string());
                        repositories.push(repository);
                    }
                }
            }
        }

        Ok(repositories)
    }
}

#[derive(Debug, Deserialize)]
struct RenvLock {
    #[serde(rename = "Packages", default)]
    packages: std::collections::BTreeMap<String, RenvPackage>,
}

#[derive(Debug, Deserialize)]
struct RenvPackage {
    #[serde(rename = "Source")]
    source: Option<String>,
    #[serde(rename = "RemoteType")]
    remote_type: Option<String>,
    #[serde(rename = "RemoteHost")]
    remote_host: Option<String>,
    #[serde(rename = "RemoteRepo")]
    remote_repo: Option<String>,
    #[serde(rename = "RemoteUrl")]
    remote_url: Option<String>,
    #[serde(rename = "Repository")]
    repository: Option<String>,
    #[serde(alias = "RemoteUsername", alias = "RemoteOwner", alias = "RemoteUser")]
    remote_owner: Option<String>,
    #[serde(rename = "URL")]
    url: Option<String>,
    #[serde(rename = "BugReports")]
    bug_reports: Option<String>,
}

impl RenvPackage {
    fn github_owner_repo(&self) -> Option<(String, String)> {
        if !self.is_github_source() {
            return None;
        }

        if let Some((owner, repo)) = self.owner_repo_from_remote_fields() {
            return Some((owner, repo));
        }

        if let Some(url) = self.remote_url.as_deref().or(self.repository.as_deref()) {
            if let Some((owner, repo)) = owner_repo_from_url(url) {
                return Some((owner, repo));
            }
        }

        for urls in [self.url.as_deref(), self.bug_reports.as_deref()]
            .into_iter()
            .flatten()
        {
            for candidate in urls.split([',', ';']) {
                let candidate = candidate.trim();
                if candidate.is_empty() {
                    continue;
                }
                if let Some((owner, repo)) = owner_repo_from_url(candidate) {
                    return Some((owner, repo));
                }
            }
        }

        None
    }

    fn is_github_source(&self) -> bool {
        self.remote_type
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case("github"))
            || self
                .source
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case("github"))
            || self
                .remote_host
                .as_deref()
                .is_some_and(|value| value.contains("github.com"))
            || self
                .remote_url
                .as_deref()
                .is_some_and(|value| value.contains("github.com"))
            || self
                .repository
                .as_deref()
                .is_some_and(|value| value.contains("github.com"))
            || self
                .url
                .as_deref()
                .is_some_and(|value| value.contains("github.com"))
            || self
                .bug_reports
                .as_deref()
                .is_some_and(|value| value.contains("github.com"))
    }

    fn owner_repo_from_remote_fields(&self) -> Option<(String, String)> {
        let repo = self.remote_repo.as_deref()?.trim().trim_end_matches(".git");
        if repo.is_empty() {
            return None;
        }

        if let Some(owner) = self.remote_owner.as_deref() {
            let owner = owner.trim();
            if owner.is_empty() {
                return None;
            }
            return Some((owner.to_string(), repo.to_string()));
        }

        if let Some((owner, repo)) = repo.split_once('/') {
            let owner = owner.trim();
            let repo = repo.trim();
            if !owner.is_empty() && !repo.is_empty() {
                return Some((owner.to_string(), repo.to_string()));
            }
        }

        None
    }
}

fn owner_repo_from_url(input: &str) -> Option<(String, String)> {
    if let Some(repo) = parse_github_repository(input) {
        return Some((repo.owner, repo.name));
    }

    let parsed = Url::parse(input).ok()?;
    match parsed.host_str()? {
        "api.github.com" => {
            let mut segments = parsed.path_segments()?;
            if segments.next()? != "repos" {
                return None;
            }
            let owner = segments.next()?.to_string();
            let repo = segments.next()?.to_string();
            Some((owner, repo))
        }
        "codeload.github.com" | "github.com" => {
            let mut segments = parsed.path_segments()?;
            let owner = segments.next()?.to_string();
            let repo = segments.next()?.to_string();
            let repo = repo.trim_end_matches(".git").to_string();
            Some((owner, repo))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_github_packages() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("renv.lock"),
            json!({
                "Packages": {
                    "pkg": {
                        "Package": "pkg",
                        "Version": "1.0.0",
                        "Source": "GitHub",
                        "RemoteType": "github",
                        "RemoteUsername": "r-lib",
                        "RemoteRepo": "pkg"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = RenvDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "r-lib");
        assert_eq!(repos[0].name, "pkg");
        assert_eq!(repos[0].via.as_deref(), Some("renv.lock"));
    }

    #[test]
    fn skips_non_github_packages() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("renv.lock"),
            json!({
                "Packages": {
                    "dplyr": {
                        "Package": "dplyr",
                        "Version": "1.0.0",
                        "Source": "CRAN",
                        "Repository": "https://cran.r-project.org"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = RenvDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }

    #[test]
    fn parses_owner_repo_from_api_urls() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("renv.lock"),
            json!({
                "Packages": {
                    "pkg": {
                        "Package": "pkg",
                        "Version": "1.0.0",
                        "Source": "GitHub",
                        "RemoteType": "github",
                        "RemoteUrl": "https://api.github.com/repos/acme/widget/tarball/HEAD"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = RenvDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "acme");
        assert_eq!(repos[0].name, "widget");
    }

    #[test]
    fn falls_back_to_package_urls() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("renv.lock"),
            json!({
                "Packages": {
                    "pkg": {
                        "Package": "pkg",
                        "Version": "1.0.0",
                        "Source": "Repository",
                        "URL": "https://example.com/docs, https://github.com/example/pkg"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = RenvDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "pkg");
    }

    #[test]
    fn falls_back_to_bug_report_urls() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("renv.lock"),
            json!({
                "Packages": {
                    "pkg": {
                        "Package": "pkg",
                        "Version": "1.0.0",
                        "Source": "Repository",
                        "BugReports": "https://github.com/example/pkg/issues"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = RenvDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "pkg");
    }
}
