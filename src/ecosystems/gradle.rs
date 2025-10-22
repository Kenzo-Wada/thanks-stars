use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum GradleDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Default)]
pub struct GradleDiscoverer;

impl GradleDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, GradleDiscoveryError> {
        let mut repositories = Vec::new();
        let mut seen = BTreeSet::new();

        collect_from_lockfile(project_root, &mut repositories, &mut seen)?;
        collect_from_build_file(project_root, "build.gradle", &mut repositories, &mut seen)?;
        collect_from_build_file(
            project_root,
            "build.gradle.kts",
            &mut repositories,
            &mut seen,
        )?;
        collect_from_build_file(
            project_root,
            "settings.gradle",
            &mut repositories,
            &mut seen,
        )?;
        collect_from_build_file(
            project_root,
            "settings.gradle.kts",
            &mut repositories,
            &mut seen,
        )?;

        Ok(repositories)
    }
}

fn collect_from_lockfile(
    project_root: &Path,
    repositories: &mut Vec<Repository>,
    seen: &mut BTreeSet<(String, String)>,
) -> Result<(), GradleDiscoveryError> {
    let path = project_root.join("gradle.lockfile");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(GradleDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let coordinate = trimmed.split('=').next().unwrap_or(trimmed);
        if let Some(repo) = repository_from_coordinate(coordinate) {
            push_repository(repo, "gradle.lockfile", repositories, seen);
        }
    }

    Ok(())
}

fn collect_from_build_file(
    project_root: &Path,
    file: &str,
    repositories: &mut Vec<Repository>,
    seen: &mut BTreeSet<(String, String)>,
) -> Result<(), GradleDiscoveryError> {
    let path = project_root.join(file);
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(GradleDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let regex = Regex::new(r#"['\"]([^:'\"]+):([^:'\"]+):[^'\"]+['\"]"#).unwrap();
    for captures in regex.captures_iter(&content) {
        let group = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        let artifact = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
        if let Some(repo) = repository_from_group_artifact(group, artifact) {
            push_repository(repo, file, repositories, seen);
        }
    }

    Ok(())
}

fn repository_from_coordinate(coordinate: &str) -> Option<Repository> {
    let mut parts = coordinate.split(':');
    let group = parts.next()?.trim();
    let artifact = parts.next()?.trim();
    repository_from_group_artifact(group, artifact)
}

fn repository_from_group_artifact(group: &str, artifact: &str) -> Option<Repository> {
    let owner = if let Some(rest) = group.strip_prefix("com.github.") {
        rest.split('.').next()?.to_string()
    } else if let Some(rest) = group.strip_prefix("io.github.") {
        rest.split('.').next()?.to_string()
    } else {
        return None;
    };

    parse_github_repository(&format!("https://github.com/{owner}/{artifact}"))
}

fn push_repository(
    mut repo: Repository,
    via: &str,
    repositories: &mut Vec<Repository>,
    seen: &mut BTreeSet<(String, String)>,
) {
    if seen.insert((repo.owner.clone(), repo.name.clone())) {
        repo.via = Some(via.to_string());
        repositories.push(repo);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_gradle_lockfile() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("gradle.lockfile"),
            "com.github.owner:artifact:1.2.3=sha\nio.github.team:library:2.0.0=sha",
        )
        .unwrap();

        let discoverer = GradleDiscoverer::new();
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "artifact");
        assert_eq!(repos[0].via.as_deref(), Some("gradle.lockfile"));
        assert_eq!(repos[1].name, "library");
    }

    #[test]
    fn discovers_repositories_from_build_files() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("build.gradle"),
            "dependencies { implementation 'com.github.user:repo:1.0.0' }",
        )
        .unwrap();

        let discoverer = GradleDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "repo");
        assert_eq!(repos[0].via.as_deref(), Some("build.gradle"));
    }
}
