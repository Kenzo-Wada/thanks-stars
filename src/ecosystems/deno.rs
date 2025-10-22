use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;
use url::Url;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum DenoDiscoveryError {
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
pub struct DenoDiscoverer;

impl DenoDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, DenoDiscoveryError> {
        let lock_path = project_root.join("deno.lock");
        let content = match fs::read_to_string(&lock_path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(err) => {
                return Err(DenoDiscoveryError::Io {
                    path: lock_path.display().to_string(),
                    source: err,
                });
            }
        };

        let lock: Value =
            serde_json::from_str(&content).map_err(|err| DenoDiscoveryError::Json {
                path: lock_path.display().to_string(),
                source: err,
            })?;

        let mut repositories = Vec::new();
        let mut seen = BTreeSet::new();
        collect_repositories(&lock, &mut repositories, &mut seen);

        for repo in &mut repositories {
            repo.via = Some("deno.lock".to_string());
        }

        Ok(repositories)
    }
}

fn collect_repositories(
    value: &Value,
    repos: &mut Vec<Repository>,
    seen: &mut BTreeSet<(String, String)>,
) {
    match value {
        Value::String(value) => {
            if let Some(repo) = repository_from_value(value) {
                if seen.insert((repo.owner.clone(), repo.name.clone())) {
                    repos.push(repo);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_repositories(item, repos, seen);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_repositories(item, repos, seen);
            }
        }
        _ => {}
    }
}

fn repository_from_value(value: &str) -> Option<Repository> {
    if let Some(repo) = parse_github_repository(value) {
        return Some(repo);
    }

    if let Ok(url) = Url::parse(value) {
        match url.host_str() {
            Some("raw.githubusercontent.com") | Some("codeload.github.com") => {
                let mut segments = url
                    .path_segments()
                    .map(|segments| segments.filter(|segment| !segment.is_empty()))?;
                let owner = segments.next()?;
                let repo = segments.next()?;
                return parse_github_repository(&format!("https://github.com/{owner}/{repo}"));
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_deno_lock() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.lock"),
            json!({
                "version": "3",
                "remote": {
                    "https://raw.githubusercontent.com/example/repo/main/mod.ts": {
                        "specifier": "https://raw.githubusercontent.com/example/repo/main/mod.ts"
                    },
                    "https://github.com/owner/another/blob/main/mod.ts": {
                        "specifier": "https://github.com/owner/another/raw/main/mod.ts"
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = DenoDiscoverer::new();
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "another");
        assert_eq!(repos[0].via.as_deref(), Some("deno.lock"));
        assert_eq!(repos[1].name, "repo");
    }

    #[test]
    fn returns_empty_when_lock_missing() {
        let dir = tempdir().unwrap();
        let discoverer = DenoDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }
}
