use std::path::Path;
use std::thread;

use crate::ecosystems::{
    CargoDiscoverer, CargoDiscoveryError, CommandMetadataFetcher, GoDiscoverer, GoDiscoveryError,
    NodeDiscoverer, NodeDiscoveryError, PythonDiscoveryError, PythonPipDiscoverer,
    PythonUvDiscoverer,
};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repository {
    pub owner: String,
    pub name: String,
    pub url: String,
    pub via: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Node,
    Cargo,
    Go,
    PythonUv,
    PythonPip,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error(transparent)]
    Node(#[from] NodeDiscoveryError),
    #[error(transparent)]
    Cargo(#[from] CargoDiscoveryError),
    #[error(transparent)]
    Go(#[from] GoDiscoveryError),
    #[error(transparent)]
    Python(#[from] PythonDiscoveryError),
}

pub trait Discoverer {
    fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, DiscoveryError>;
}

pub fn detect_frameworks(project_root: &Path) -> Vec<Framework> {
    let mut frameworks = Vec::new();
    if project_root.join("package.json").exists() {
        frameworks.push(Framework::Node);
    }
    if project_root.join("Cargo.toml").exists() {
        frameworks.push(Framework::Cargo);
    }
    if project_root.join("go.mod").exists() {
        frameworks.push(Framework::Go);
    }
    if project_root.join("uv.lock").exists() {
        frameworks.push(Framework::PythonUv);
    }
    if project_root.join("requirements.txt").exists() {
        frameworks.push(Framework::PythonPip);
    }
    frameworks
}

pub fn discover_for_frameworks(
    project_root: &Path,
    frameworks: &[Framework],
) -> Result<Vec<Repository>, DiscoveryError> {
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(frameworks.len());

        for (index, framework) in frameworks.iter().copied().enumerate() {
            handles.push(scope.spawn(
                move || -> Result<(usize, Vec<Repository>), DiscoveryError> {
                    let repositories = match framework {
                        Framework::Node => {
                            let discoverer = NodeDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Cargo => {
                            let discoverer = CargoDiscoverer::new(CommandMetadataFetcher);
                            discoverer.discover(project_root)?
                        }
                        Framework::Go => {
                            let discoverer = GoDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::PythonUv => {
                            let discoverer = PythonUvDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::PythonPip => {
                            let discoverer = PythonPipDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                    };

                    Ok((index, repositories))
                },
            ));
        }

        let mut ordered = Vec::with_capacity(handles.len());
        for handle in handles {
            let result = handle.join().expect("framework discovery task panicked")?;
            ordered.push(result);
        }

        ordered.sort_by_key(|(index, _)| *index);

        let mut repositories = Vec::new();
        for (_, mut repos) in ordered {
            repositories.append(&mut repos);
        }

        Ok(repositories)
    })
}

pub fn parse_github_repository(input: &str) -> Option<Repository> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("github:") {
        return parse_owner_repo(rest.trim());
    }

    let without_git = trimmed.strip_prefix("git+").unwrap_or(trimmed);

    if let Ok(url) = Url::parse(without_git) {
        if url.scheme() == "file" {
            return None;
        }
        if matches!(url.host_str(), Some("github.com")) {
            let segments = url
                .path_segments()
                .map(|segments| segments.filter(|segment| !segment.is_empty()));
            if let Some(mut segments) = segments {
                let owner = segments.next()?;
                let repo = segments.next()?;
                return build_repository(owner, repo);
            }
        }
    } else if let Some(repo) = parse_owner_repo(without_git) {
        return Some(repo);
    }

    if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest);
    }

    None
}

fn parse_owner_repo(input: &str) -> Option<Repository> {
    let mut parts = input.trim_matches('/').split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    build_repository(owner, repo)
}

fn build_repository(owner: &str, repo: &str) -> Option<Repository> {
    let repo = repo.trim_end_matches(".git");
    if repo.is_empty() || owner.is_empty() {
        return None;
    }
    Some(Repository {
        owner: owner.to_string(),
        name: repo.to_string(),
        url: format!("https://github.com/{owner}/{repo}"),
        via: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url() {
        let repo = parse_github_repository("https://github.com/owner/repo").unwrap();
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn parses_git_plus_url_and_strips_git_suffix() {
        let repo = parse_github_repository("git+https://github.com/owner/repo.git").unwrap();
        assert_eq!(repo.owner, "owner");
        assert_eq!(repo.name, "repo");
    }

    #[test]
    fn parses_owner_repo_shorthand() {
        let repo = parse_github_repository("owner/repo").unwrap();
        assert_eq!(repo.url, "https://github.com/owner/repo");
    }

    #[test]
    fn returns_none_for_non_github_url() {
        assert!(parse_github_repository("https://example.com/owner/repo").is_none());
    }
}
