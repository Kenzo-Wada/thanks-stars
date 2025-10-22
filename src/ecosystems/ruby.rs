use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum RubyDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Default)]
pub struct RubyDiscoverer;

impl RubyDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, RubyDiscoveryError> {
        let lock_path = project_root.join("Gemfile.lock");
        let content = match fs::read_to_string(&lock_path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(RubyDiscoveryError::Io {
                    path: lock_path.display().to_string(),
                    source: err,
                })
            }
        };

        let mut repositories = Vec::new();
        let mut seen = BTreeSet::new();
        let mut in_git_section = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                in_git_section = false;
                continue;
            }
            if trimmed == "GIT" {
                in_git_section = true;
                continue;
            }
            if in_git_section {
                if let Some(rest) = trimmed.strip_prefix("remote:") {
                    let remote = rest.trim();
                    if let Some(mut repo) = parse_github_repository(remote) {
                        if seen.insert((repo.owner.clone(), repo.name.clone())) {
                            repo.via = Some("Gemfile.lock".to_string());
                            repositories.push(repo);
                        }
                    }
                }
            }
        }

        Ok(repositories)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_git_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Gemfile.lock"),
            "GIT\n  remote: https://github.com/example/repo.git\n  revision: abc123\n\nGEM\n  remote: https://rubygems.org/\n  specs:\n    rails (7.0.0)\n",
        )
        .unwrap();

        let discoverer = RubyDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "repo");
        assert_eq!(repos[0].via.as_deref(), Some("Gemfile.lock"));
    }

    #[test]
    fn returns_empty_when_lock_missing() {
        let dir = tempdir().unwrap();
        let discoverer = RubyDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }
}
