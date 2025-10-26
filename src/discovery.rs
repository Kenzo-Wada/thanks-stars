use std::path::Path;
use std::thread;

use crate::ecosystems::{
    CargoDiscoverer, CargoDiscoveryError, CommandMetadataFetcher, ComposerDiscoverer,
    ComposerDiscoveryError, DartDiscoverer, DartDiscoveryError, DenoDiscoverer, DenoDiscoveryError,
    GoDiscoverer, GoDiscoveryError, GradleDiscoverer, GradleDiscoveryError, MavenDiscoverer,
    MavenDiscoveryError, NodeDiscoverer, NodeDiscoveryError, PythonDiscoverer,
    PythonDiscoveryError, RenvDiscoverer, RenvDiscoveryError, RubyDiscoverer, RubyDiscoveryError,
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
    Deno,
    Cargo,
    Go,
    Dart,
    Composer,
    Ruby,
    Python,
    Gradle,
    Maven,
    Renv,
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error(transparent)]
    Node(Box<NodeDiscoveryError>),
    #[error(transparent)]
    Deno(Box<DenoDiscoveryError>),
    #[error(transparent)]
    Cargo(Box<CargoDiscoveryError>),
    #[error(transparent)]
    Go(Box<GoDiscoveryError>),
    #[error(transparent)]
    Dart(Box<DartDiscoveryError>),
    #[error(transparent)]
    Composer(Box<ComposerDiscoveryError>),
    #[error(transparent)]
    Ruby(Box<RubyDiscoveryError>),
    #[error(transparent)]
    Python(Box<PythonDiscoveryError>),
    #[error(transparent)]
    Gradle(Box<GradleDiscoveryError>),
    #[error(transparent)]
    Maven(Box<MavenDiscoveryError>),
    #[error(transparent)]
    Renv(Box<RenvDiscoveryError>),
}

macro_rules! impl_from_discovery_error {
    ($variant:ident, $ty:ty) => {
        impl From<$ty> for DiscoveryError {
            fn from(value: $ty) -> Self {
                Self::$variant(Box::new(value))
            }
        }
    };
}

impl_from_discovery_error!(Node, NodeDiscoveryError);
impl_from_discovery_error!(Deno, DenoDiscoveryError);
impl_from_discovery_error!(Cargo, CargoDiscoveryError);
impl_from_discovery_error!(Go, GoDiscoveryError);
impl_from_discovery_error!(Dart, DartDiscoveryError);
impl_from_discovery_error!(Composer, ComposerDiscoveryError);
impl_from_discovery_error!(Ruby, RubyDiscoveryError);
impl_from_discovery_error!(Python, PythonDiscoveryError);
impl_from_discovery_error!(Gradle, GradleDiscoveryError);
impl_from_discovery_error!(Maven, MavenDiscoveryError);
impl_from_discovery_error!(Renv, RenvDiscoveryError);

pub trait Discoverer {
    fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, DiscoveryError>;
}

pub fn detect_frameworks(project_root: &Path) -> Vec<Framework> {
    let mut frameworks = Vec::new();
    if project_root.join("package.json").exists() {
        frameworks.push(Framework::Node);
    }
    if ["deno.lock", "deno.json", "deno.jsonc", "jsr.json"]
        .iter()
        .any(|file| project_root.join(file).exists())
    {
        frameworks.push(Framework::Deno);
    }
    if project_root.join("Cargo.toml").exists() {
        frameworks.push(Framework::Cargo);
    }
    if project_root.join("go.mod").exists() {
        frameworks.push(Framework::Go);
    }
    if project_root.join("pubspec.yaml").exists() {
        frameworks.push(Framework::Dart);
    }
    if project_root.join("composer.lock").exists() || project_root.join("composer.json").exists() {
        frameworks.push(Framework::Composer);
    }
    if project_root.join("Gemfile").exists() || project_root.join("Gemfile.lock").exists() {
        frameworks.push(Framework::Ruby);
    }
    if project_root.join("pyproject.toml").exists()
        || project_root.join("requirements.txt").exists()
        || project_root.join("Pipfile").exists()
        || project_root.join("Pipfile.lock").exists()
        || project_root.join("uv.lock").exists()
    {
        frameworks.push(Framework::Python);
    }
    if project_root.join("gradle.lockfile").exists()
        || project_root.join("build.gradle").exists()
        || project_root.join("build.gradle.kts").exists()
    {
        frameworks.push(Framework::Gradle);
    }
    if project_root.join("pom.xml").exists() {
        frameworks.push(Framework::Maven);
    }
    if project_root.join("renv.lock").exists() {
        frameworks.push(Framework::Renv);
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
                        Framework::Deno => {
                            let discoverer = DenoDiscoverer::new();
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
                        Framework::Dart => {
                            let discoverer = DartDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Composer => {
                            let discoverer = ComposerDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Ruby => {
                            let discoverer = RubyDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Python => {
                            let discoverer = PythonDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Gradle => {
                            let discoverer = GradleDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Maven => {
                            let discoverer = MavenDiscoverer::new();
                            discoverer.discover(project_root)?
                        }
                        Framework::Renv => {
                            let discoverer = RenvDiscoverer::new();
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
