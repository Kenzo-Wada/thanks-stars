use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum MavenDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    Xml {
        path: String,
        #[source]
        source: quick_xml::Error,
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
struct MavenCoordinate {
    group: String,
    artifact: String,
    version: String,
}

type DependencyMap = BTreeMap<MavenCoordinate, BTreeSet<String>>;

pub struct MavenDiscoverer<F: MavenFetcher> {
    fetcher: F,
}

impl Default for MavenDiscoverer<HttpMavenClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl MavenDiscoverer<HttpMavenClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpMavenClient::new(),
        }
    }
}

impl<F: MavenFetcher> MavenDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, MavenDiscoveryError> {
        let mut dependencies: DependencyMap = BTreeMap::new();
        collect_pom_dependencies(project_root, project_root, &mut dependencies)?;

        let mut repositories = Vec::new();

        for (coord, vias) in dependencies {
            let Some(project) = self
                .fetcher
                .fetch(&coord.group, &coord.artifact, &coord.version)
                .map_err(|source| {
                    MavenDiscoveryError::Maven(Box::new(MavenDependencyError {
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
                        repository.via = Some("pom.xml".to_string());
                    }
                    repositories.push(repository);
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

fn collect_pom_dependencies(
    project_root: &Path,
    module_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), MavenDiscoveryError> {
    let pom_path = module_root.join("pom.xml");
    let via = pom_path
        .strip_prefix(project_root)
        .unwrap_or(&pom_path)
        .to_string_lossy()
        .replace('\\', "/");

    let content = match fs::read_to_string(&pom_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(MavenDiscoveryError::Io {
                path: pom_path.display().to_string(),
                source: err,
            })
        }
    };

    let parse_result = parse_pom(&content).map_err(|source| MavenDiscoveryError::Xml {
        path: pom_path.display().to_string(),
        source,
    })?;

    for coordinate in parse_result.dependencies {
        insert_dependency(
            dependencies,
            &coordinate.group,
            &coordinate.artifact,
            &coordinate.version,
            &via,
        );
    }

    let current_pom_normalized = normalize_path(pom_path.clone());

    for module in parse_result.modules {
        if module.trim().is_empty() {
            continue;
        }
        let module_root = normalize_module_path(module_root, &module);
        let module_pom = module_root.join("pom.xml");
        if normalize_path(module_pom) == current_pom_normalized {
            continue;
        }
        collect_pom_dependencies(project_root, &module_root, dependencies)?;
    }

    Ok(())
}

struct PomParseResult {
    dependencies: Vec<MavenCoordinate>,
    modules: Vec<String>,
}

fn parse_pom(pom: &str) -> Result<PomParseResult, quick_xml::Error> {
    let mut reader = Reader::from_str(pom);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut modules = Vec::new();
    let mut dependencies = Vec::new();
    let mut state: Option<DependencyState> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(element) => {
                let name = reader
                    .decoder()
                    .decode(element.name().as_ref())?
                    .into_owned();

                let parent = stack.last().map(|s| s.as_str());
                if name == "dependency" && parent == Some("dependencies") {
                    let in_dependency_management =
                        stack.iter().any(|s| s == "dependencyManagement");
                    let in_plugin = stack.iter().any(|s| s == "plugin");
                    if in_dependency_management || in_plugin {
                        state = Some(DependencyState::Skip);
                    } else {
                        state = Some(DependencyState::Capture(DependencyBuilder::default()));
                    }
                }

                stack.push(name);
            }
            Event::End(element) => {
                let name = reader
                    .decoder()
                    .decode(element.name().as_ref())?
                    .into_owned();

                if name == "dependency" {
                    if let Some(DependencyState::Capture(builder)) = state.take() {
                        if let (Some(group), Some(artifact), Some(version)) =
                            (builder.group, builder.artifact, builder.version)
                        {
                            dependencies.push(MavenCoordinate {
                                group,
                                artifact,
                                version,
                            });
                        }
                    } else {
                        state = None;
                    }
                }

                stack.pop();
            }
            Event::Text(text) => {
                let value = text.decode()?.into_owned();
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    continue;
                }

                if let Some(current) = stack.last() {
                    if current == "module" {
                        let parent = stack.iter().rev().nth(1).map(|s| s.as_str());
                        if matches!(parent, Some("modules")) {
                            modules.push(trimmed.to_string());
                        }
                    }
                }

                if let Some(DependencyState::Capture(builder)) = state.as_mut() {
                    if let Some(current) = stack.last() {
                        match current.as_str() {
                            "groupId" => builder.group = Some(trimmed.to_string()),
                            "artifactId" => builder.artifact = Some(trimmed.to_string()),
                            "version" => builder.version = Some(trimmed.to_string()),
                            _ => {}
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(PomParseResult {
        dependencies,
        modules,
    })
}

enum DependencyState {
    Capture(DependencyBuilder),
    Skip,
}

#[derive(Default)]
struct DependencyBuilder {
    group: Option<String>,
    artifact: Option<String>,
    version: Option<String>,
}

fn normalize_module_path(base: &Path, module: &str) -> PathBuf {
    let trimmed = module.trim();
    if trimmed.is_empty() {
        return base.to_path_buf();
    }

    let path = PathBuf::from(trimmed);
    if path.is_relative() {
        normalize_path(base.join(path))
    } else {
        normalize_path(path)
    }
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }

    normalized
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
    if version.contains('$') || version.contains('{') || version.contains('}') {
        return;
    }
    if version.contains('[') || version.contains('(') {
        return;
    }

    let coordinate = MavenCoordinate {
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
    fn discovers_repositories_from_pom() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pom.xml"),
            r#"
            <project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId>
              <artifactId>app</artifactId>
              <version>1.0.0</version>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>library</artifactId>
                  <version>1.2.3</version>
                </dependency>
              </dependencies>
            </project>
            "#,
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
            MavenDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "library");
        assert_eq!(repos[0].via.as_deref(), Some("pom.xml"));
    }

    #[test]
    fn discovers_repositories_from_modules() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("module-a")).unwrap();
        fs::write(
            dir.path().join("pom.xml"),
            r#"
            <project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId>
              <artifactId>parent</artifactId>
              <version>1.0.0</version>
              <modules>
                <module>module-a</module>
              </modules>
            </project>
            "#,
        )
        .unwrap();
        fs::write(
            dir.path().join("module-a/pom.xml"),
            r#"
            <project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId>
              <artifactId>module-a</artifactId>
              <version>1.0.0</version>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>library</artifactId>
                  <version>1.2.3</version>
                </dependency>
              </dependencies>
            </project>
            "#,
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
                </project>
                "#,
            );
        });

        let discoverer =
            MavenDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].via.as_deref(), Some("module-a/pom.xml"));
    }

    #[test]
    fn skips_dependencies_with_property_versions() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pom.xml"),
            r#"
            <project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId>
              <artifactId>app</artifactId>
              <version>1.0.0</version>
              <properties>
                <library.version>1.2.3</library.version>
              </properties>
              <dependencies>
                <dependency>
                  <groupId>com.example</groupId>
                  <artifactId>library</artifactId>
                  <version>${library.version}</version>
                </dependency>
              </dependencies>
            </project>
            "#,
        )
        .unwrap();

        let server = MockServer::start();
        let discoverer =
            MavenDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }

    #[test]
    fn skips_plugin_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pom.xml"),
            r#"
            <project>
              <modelVersion>4.0.0</modelVersion>
              <groupId>com.example</groupId>
              <artifactId>app</artifactId>
              <version>1.0.0</version>
              <build>
                <plugins>
                  <plugin>
                    <groupId>com.example</groupId>
                    <artifactId>plugin</artifactId>
                    <version>1.0.0</version>
                    <dependencies>
                      <dependency>
                        <groupId>com.example</groupId>
                        <artifactId>library</artifactId>
                        <version>1.2.3</version>
                      </dependency>
                    </dependencies>
                  </plugin>
                </plugins>
              </build>
            </project>
            "#,
        )
        .unwrap();

        let server = MockServer::start();
        let discoverer =
            MavenDiscoverer::with_fetcher(HttpMavenClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }
}
