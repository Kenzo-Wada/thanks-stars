pub mod config;
pub mod discovery;
pub mod ecosystems;
pub mod github;
pub mod http;

use std::collections::HashSet;
use std::path::Path;

use discovery::{DiscoveryError, Framework, Repository};
use github::GitHubApi;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Discovery(Box<DiscoveryError>),
    #[error(transparent)]
    GitHub(#[from] github::GitHubError),
    #[error("no supported package managers found in project root {0}")]
    NoFrameworks(String),
}

impl From<DiscoveryError> for RunError {
    fn from(value: DiscoveryError) -> Self {
        Self::Discovery(Box::new(value))
    }
}

#[derive(Debug, Clone)]
pub struct StarredRepository {
    pub repository: Repository,
    pub already_starred: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RunSummary {
    pub starred: Vec<StarredRepository>,
}

pub trait RunEventHandler {
    fn on_start(&mut self, _total: usize) {}
    fn on_starred(
        &mut self,
        _repo: &Repository,
        _already_starred: bool,
        _index: usize,
        _total: usize,
    ) {
    }
    fn on_complete(&mut self, _summary: &RunSummary) {}
}

#[derive(Default)]
struct NoopHandler;

impl RunEventHandler for NoopHandler {}

pub fn run(project_root: &Path, api: &dyn GitHubApi) -> Result<RunSummary, RunError> {
    let frameworks = discovery::detect_frameworks(project_root);
    if frameworks.is_empty() {
        return Err(RunError::NoFrameworks(project_root.display().to_string()));
    }

    run_with_frameworks_and_handler(project_root, &frameworks, api, &mut NoopHandler)
}

pub fn run_with_frameworks(
    project_root: &Path,
    frameworks: &[Framework],
    api: &dyn GitHubApi,
) -> Result<RunSummary, RunError> {
    if frameworks.is_empty() {
        return Err(RunError::NoFrameworks(project_root.display().to_string()));
    }
    run_with_frameworks_and_handler(project_root, frameworks, api, &mut NoopHandler)
}

pub fn run_with_handler(
    project_root: &Path,
    api: &dyn GitHubApi,
    handler: &mut impl RunEventHandler,
) -> Result<RunSummary, RunError> {
    let frameworks = discovery::detect_frameworks(project_root);
    if frameworks.is_empty() {
        return Err(RunError::NoFrameworks(project_root.display().to_string()));
    }

    run_with_frameworks_and_handler(project_root, &frameworks, api, handler)
}

pub fn run_with_frameworks_and_handler(
    project_root: &Path,
    frameworks: &[Framework],
    api: &dyn GitHubApi,
    handler: &mut impl RunEventHandler,
) -> Result<RunSummary, RunError> {
    let repos = discovery::discover_for_frameworks(project_root, frameworks)?;

    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for repo in repos {
        if seen.insert((repo.owner.clone(), repo.name.clone())) {
            unique.push(repo);
        }
    }

    handler.on_start(unique.len());

    let total = unique.len();
    let mut starred = Vec::new();
    for (index, repo) in unique.into_iter().enumerate() {
        let already_starred = api.viewer_has_starred(&repo.owner, &repo.name)?;
        if !already_starred {
            api.star(&repo.owner, &repo.name)?;
        }
        handler.on_starred(&repo, already_starred, index + 1, total);
        starred.push(StarredRepository {
            repository: repo,
            already_starred,
        });
    }

    let summary = RunSummary { starred };
    handler.on_complete(&summary);

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::Framework;
    use crate::github::GitHubError;
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;
    use tempfile::tempdir;

    struct MockGitHub {
        calls: RefCell<Vec<(String, String)>>,
        starred: RefCell<Vec<(String, String)>>,
    }

    impl MockGitHub {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                starred: RefCell::new(Vec::new()),
            }
        }
    }

    impl GitHubApi for MockGitHub {
        fn viewer_has_starred(&self, owner: &str, repo: &str) -> Result<bool, GitHubError> {
            Ok(self
                .starred
                .borrow()
                .iter()
                .any(|(o, r)| o == owner && r == repo))
        }

        fn star(&self, owner: &str, repo: &str) -> Result<(), GitHubError> {
            self.calls
                .borrow_mut()
                .push((owner.to_string(), repo.to_string()));
            self.starred
                .borrow_mut()
                .push((owner.to_string(), repo.to_string()));
            Ok(())
        }
    }

    #[test]
    fn stars_unique_repositories_once() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            json!({
                "dependencies": {
                    "dep-one": "^1.0.0",
                    "dep-two": "^1.0.0"
                }
            })
            .to_string(),
        )
        .unwrap();

        let dep_one = dir.path().join("node_modules/dep-one");
        let dep_two = dir.path().join("node_modules/dep-two");
        fs::create_dir_all(&dep_one).unwrap();
        fs::create_dir_all(&dep_two).unwrap();

        let package_json = json!({ "repository": "https://github.com/example/repo" }).to_string();
        fs::write(dep_one.join("package.json"), &package_json).unwrap();
        fs::write(dep_two.join("package.json"), &package_json).unwrap();

        let mock = MockGitHub::new();
        let summary = run_with_frameworks(dir.path(), &[Framework::Node], &mock).unwrap();

        assert_eq!(summary.starred.len(), 1);
        assert_eq!(summary.starred[0].repository.owner, "example");
        assert_eq!(summary.starred[0].repository.name, "repo");
        assert!(!summary.starred[0].already_starred);
        let calls = mock.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], ("example".to_string(), "repo".to_string()));
    }
}
