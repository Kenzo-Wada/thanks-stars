use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;

use crate::discovery::{parse_github_repository, Repository};

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

#[derive(Debug, thiserror::Error)]
#[error("{group}:{artifact}:{version}: {source}")]
pub struct MavenDependencyError {
    pub group: String,
    pub artifact: String,
    pub version: String,
    #[source]
    pub source: MavenError,
}

#[derive(Debug, thiserror::Error)]
pub enum MavenError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
    #[error("failed to parse POM: {source}")]
    Xml {
        #[from]
        source: quick_xml::Error,
    },
}

pub trait MavenFetcher {
    fn fetch(
        &self,
        group: &str,
        artifact: &str,
        version: &str,
    ) -> Result<Option<MavenProject>, MavenError>;
}

#[derive(Clone)]
pub struct HttpMavenClient {
    client: Client,
    base_url: String,
}

impl Default for HttpMavenClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpMavenClient {
    const DEFAULT_BASE_URL: &'static str = "https://repo1.maven.org/maven2";

    pub fn new() -> Self {
        Self {
            client: Client::new(),
            base_url: Self::DEFAULT_BASE_URL.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
        }
    }
}

impl MavenFetcher for HttpMavenClient {
    fn fetch(
        &self,
        group: &str,
        artifact: &str,
        version: &str,
    ) -> Result<Option<MavenProject>, MavenError> {
        let group_path = group.replace('.', "/");
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/{group_path}/{artifact}/{version}/{artifact}-{version}.pom");
        let response = self
            .client
            .get(&url)
            .header(ACCEPT, "application/xml")
            .send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(MavenError::UnexpectedStatus { status }),
            _ => {
                let text = response.text()?;
                let project = MavenProject::from_pom(&text)?;
                Ok(Some(project))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct MavenProject {
    urls: Vec<String>,
}

impl MavenProject {
    fn from_pom(pom: &str) -> Result<Self, MavenError> {
        let mut reader = Reader::from_str(pom);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut stack: Vec<String> = Vec::new();
        let mut urls = Vec::new();

        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Start(element) => {
                    let name = reader
                        .decoder()
                        .decode(element.name().as_ref())
                        .map_err(|err| MavenError::Xml { source: err.into() })?
                        .into_owned();
                    stack.push(name);
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::Text(text) => {
                    if let Some(current) = stack.last().map(|s| s.as_str()) {
                        let value = text
                            .decode()
                            .map_err(|err| MavenError::Xml { source: err.into() })?
                            .into_owned();
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let parent = stack.iter().rev().nth(1).map(|s| s.as_str());
                        match current {
                            "url" => {
                                if matches!(parent, Some("project" | "scm")) {
                                    urls.push(trimmed.to_string());
                                }
                            }
                            "connection" | "developerConnection" => {
                                if matches!(parent, Some("scm")) {
                                    urls.push(trimmed.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }

        Ok(Self { urls })
    }

    pub fn candidate_urls(&self) -> Vec<String> {
        let mut unique = BTreeSet::new();
        let mut candidates = Vec::new();

        for raw in &self.urls {
            let mut value = raw.trim();
            if value.is_empty() {
                continue;
            }
            if let Some(rest) = value.strip_prefix("scm:") {
                value = rest;
            }
            if let Some(rest) = value.strip_prefix("git:") {
                value = rest;
            }
            if unique.insert(value.to_lowercase()) {
                candidates.push(value.to_string());
            }
        }

        candidates
    }
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
