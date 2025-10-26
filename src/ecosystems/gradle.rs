use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::discovery::{parse_github_repository, Repository};
use crate::ecosystems::maven::{HttpMavenClient, MavenDependencyError, MavenFetcher};

#[derive(Debug, thiserror::Error)]
pub enum GradleDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to fetch metadata for {0}")]
    Maven(#[from] Box<MavenDependencyError>),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct GradleCoordinate {
    group: String,
    artifact: String,
    version: String,
}

type DependencyMap = BTreeMap<GradleCoordinate, BTreeSet<String>>;

pub struct GradleDiscoverer<F: MavenFetcher> {
    fetcher: F,
}

impl Default for GradleDiscoverer<HttpMavenClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl GradleDiscoverer<HttpMavenClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpMavenClient::new(),
        }
    }
}

impl<F: MavenFetcher> GradleDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, GradleDiscoveryError> {
        let mut dependencies: DependencyMap = BTreeMap::new();

        collect_lockfile_dependencies(project_root, &mut dependencies)?;
        collect_build_dependencies(project_root, "build.gradle", &mut dependencies)?;
        collect_build_dependencies(project_root, "build.gradle.kts", &mut dependencies)?;

        let mut repositories = Vec::new();

        for (coord, vias) in dependencies {
            let Some(project) = self
                .fetcher
                .fetch(&coord.group, &coord.artifact, &coord.version)
                .map_err(|source| {
                    GradleDiscoveryError::Maven(Box::new(MavenDependencyError {
                        group: coord.group.clone(),
                        artifact: coord.artifact.clone(),
                        version: coord.version.clone(),
                        source,
                    }))
                })?
            else {
                continue;
            };

            for url in project.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(&url) {
                    if let Some(via) = vias.iter().next() {
                        repository.via = Some(via.clone());
                    } else {
                        repository.via = Some("Gradle".to_string());
                    }
                    repositories.push(repository);
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

fn collect_lockfile_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
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
        let coords = match trimmed.split_once('=') {
            Some((coords, _)) => coords.trim(),
            None => trimmed,
        };
        let coords = coords.split('@').next().unwrap_or(coords);
        let mut parts = coords.split(':');
        let (Some(group), Some(artifact), Some(version)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let version = version.split_whitespace().next().unwrap_or(version);
        insert_dependency(dependencies, group, artifact, version, "gradle.lockfile");
    }

    Ok(())
}

fn collect_build_dependencies(
    project_root: &Path,
    filename: &str,
    dependencies: &mut DependencyMap,
) -> Result<(), GradleDiscoveryError> {
    let path = project_root.join(filename);
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

    let regex = Regex::new(r#"['\"]([A-Za-z0-9_.-]+):([A-Za-z0-9_.-]+):([A-Za-z0-9+_.-]+)['\"]"#)
        .expect("valid regex");

    for capture in regex.captures_iter(&content) {
        let group = capture[1].to_string();
        let artifact = capture[2].to_string();
        let version = capture[3].to_string();
        insert_dependency(dependencies, &group, &artifact, &version, filename);
    }

    Ok(())
}

fn insert_dependency(
    dependencies: &mut DependencyMap,
    group: &str,
    artifact: &str,
    version: &str,
    via: &str,
) {
    if group.is_empty() || artifact.is_empty() || version.is_empty() {
        return;
    }
    let coordinate = GradleCoordinate {
        group: group.to_string(),
        artifact: artifact.to_string(),
        version: version.to_string(),
    };
    dependencies
        .entry(coordinate)
        .or_default()
        .insert(via.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_gradle_lockfile() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("gradle.lockfile"),
            "com.example:library:1.2.3=runtimeClasspath\n",
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/com/example/library/1.2.3/library-1.2.3.pom");
            then.status(200).body(
                r#"
                <project>
                  <url>https://github.com/example/library</url>
                  <scm>
                    <connection>scm:git:https://github.com/example/library.git</connection>
                  </scm>
                </project>
                "#,
            );
        });

        let discoverer =
            GradleDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "library");
        assert_eq!(repos[0].via.as_deref(), Some("gradle.lockfile"));
    }

    #[test]
    fn ignores_missing_metadata() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("gradle.lockfile"),
            "com.example:missing:1.0.0=runtimeClasspath\n",
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET)
                .path("/com/example/missing/1.0.0/missing-1.0.0.pom");
            then.status(404);
        });

        let discoverer =
            GradleDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }
}
