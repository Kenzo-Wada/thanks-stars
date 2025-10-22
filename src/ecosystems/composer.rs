use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum ComposerDiscoveryError {
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
pub struct ComposerDiscoverer;

impl ComposerDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, ComposerDiscoveryError> {
        let lock_path = project_root.join("composer.lock");
        let content = match fs::read_to_string(&lock_path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(ComposerDiscoveryError::Io {
                    path: lock_path.display().to_string(),
                    source: err,
                })
            }
        };

        let lock: ComposerLock =
            serde_json::from_str(&content).map_err(|source| ComposerDiscoveryError::Json {
                path: lock_path.display().to_string(),
                source,
            })?;

        let mut repositories = Vec::new();
        let mut seen = BTreeSet::new();

        for package in lock
            .packages
            .into_iter()
            .chain(lock.packages_dev.into_iter())
        {
            for candidate in package.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(candidate) {
                    if seen.insert((repository.owner.clone(), repository.name.clone())) {
                        repository.via = Some("composer.lock".to_string());
                        repositories.push(repository);
                    }
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

#[derive(Debug, Deserialize)]
struct ComposerLock {
    #[serde(default)]
    packages: Vec<ComposerPackage>,
    #[serde(rename = "packages-dev", default)]
    packages_dev: Vec<ComposerPackage>,
}

#[derive(Debug, Deserialize)]
struct ComposerPackage {
    #[serde(default)]
    source: Option<ComposerSource>,
    #[serde(default)]
    support: Option<ComposerSupport>,
    #[serde(default)]
    homepage: Option<String>,
}

impl ComposerPackage {
    fn candidate_urls(&self) -> impl Iterator<Item = &str> {
        let mut urls: Vec<&str> = Vec::new();
        if let Some(source) = &self.source {
            if let Some(url) = source.url.as_deref() {
                urls.push(url);
            }
        }
        if let Some(support) = &self.support {
            if let Some(url) = support.source.as_deref() {
                urls.push(url);
            }
        }
        if let Some(homepage) = &self.homepage {
            urls.push(homepage);
        }
        urls.into_iter()
    }
}

#[derive(Debug, Deserialize)]
struct ComposerSource {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ComposerSupport {
    #[serde(default)]
    source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_packages() {
        let dir = tempdir().unwrap();
        let lock = json!({
            "packages": [
                {
                    "name": "vendor/package",
                    "source": {
                        "type": "git",
                        "url": "https://github.com/vendor/package.git"
                    }
                }
            ],
            "packages-dev": [
                {
                    "name": "vendor/dev-package",
                    "support": {
                        "source": "https://github.com/vendor/dev-package"
                    }
                },
                {
                    "name": "vendor/homepage",
                    "homepage": "https://github.com/vendor/homepage"
                },
                {
                    "name": "vendor/non-github",
                    "homepage": "https://example.com/vendor/non-github"
                }
            ]
        });

        fs::write(dir.path().join("composer.lock"), lock.to_string()).unwrap();

        let discoverer = ComposerDiscoverer::new();
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 3);
        assert_eq!(repos[0].name, "dev-package");
        assert_eq!(repos[1].name, "homepage");
        assert_eq!(repos[2].name, "package");
        for repo in repos {
            assert_eq!(repo.via.as_deref(), Some("composer.lock"));
        }
    }

    #[test]
    fn ignores_missing_lockfile() {
        let dir = tempdir().unwrap();
        let discoverer = ComposerDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }
}
