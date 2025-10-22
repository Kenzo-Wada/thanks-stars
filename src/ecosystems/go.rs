use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum GoDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Default)]
pub struct GoDiscoverer;

impl GoDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, GoDiscoveryError> {
        let go_mod_path = project_root.join("go.mod");
        let content = fs::read_to_string(&go_mod_path).map_err(|err| GoDiscoveryError::Io {
            path: go_mod_path.display().to_string(),
            source: err,
        })?;

        let mut names = BTreeSet::new();
        parse_requirements(&content, &mut names);

        let mut repositories = Vec::new();
        for name in names {
            if let Some(mut repository) = parse_go_module(&name) {
                repository.via = Some("go.mod".to_string());
                repositories.push(repository);
            }
        }

        Ok(repositories)
    }
}

fn parse_requirements(content: &str, names: &mut BTreeSet<String>) {
    let mut in_block = false;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.starts_with("require (") {
            in_block = true;
            continue;
        }

        if in_block {
            if line == ")" {
                in_block = false;
                continue;
            }
            if let Some(name) = parse_module_name(line) {
                names.insert(name);
            }
            continue;
        }

        if line.starts_with("require ") {
            if let Some(rest) = line.strip_prefix("require ") {
                if let Some(name) = parse_module_name(rest) {
                    names.insert(name);
                }
            }
        }
    }
}

fn parse_module_name(line: &str) -> Option<String> {
    let without_comment = line.split("//").next()?.trim();
    if without_comment.is_empty() {
        return None;
    }
    without_comment
        .split_whitespace()
        .next()
        .map(|s| s.to_string())
}

fn parse_go_module(module: &str) -> Option<Repository> {
    let module = module.strip_prefix("github.com/")?;
    let mut parts = module.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    parse_github_repository(&format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_github_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/project\n\nrequire (\n    github.com/pkg/errors v0.9.1\n    golang.org/x/net v0.17.0\n    github.com/org/repo/v2 v2.0.0\n)\n",
        )
        .unwrap();

        let discoverer = GoDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        let owners: Vec<_> = repos
            .iter()
            .map(|repo| (repo.owner.as_str(), repo.name.as_str()))
            .collect();

        assert_eq!(repos.len(), 2);
        assert!(owners.contains(&("org", "repo")));
        assert!(owners.contains(&("pkg", "errors")));
    }

    #[test]
    fn skips_non_github_modules() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("go.mod"),
            "module example\n\nrequire golang.org/x/text v0.15.0\n",
        )
        .unwrap();

        let discoverer = GoDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }
}
