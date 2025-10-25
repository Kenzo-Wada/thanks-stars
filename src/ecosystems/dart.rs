use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::discovery::{parse_github_repository, Repository};

const PUBSPEC_FILE: &str = "pubspec.yaml";

#[derive(Debug, thiserror::Error)]
pub enum DartDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as YAML: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to fetch metadata for package {name}: {source}")]
    PubDev {
        name: String,
        #[source]
        source: PubDevError,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PubDevError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
}

pub trait PubDevFetcher {
    fn fetch(&self, name: &str) -> Result<Option<PubDevPackage>, PubDevError>;
}

#[derive(Clone)]
pub struct HttpPubDevClient {
    client: Client,
    base_url: String,
}

impl Default for HttpPubDevClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPubDevClient {
    const DEFAULT_BASE_URL: &'static str = "https://pub.dev/api/packages";

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

impl PubDevFetcher for HttpPubDevClient {
    fn fetch(&self, name: &str) -> Result<Option<PubDevPackage>, PubDevError> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{base}/{name}");
        let response = self
            .client
            .get(&url)
            .header(ACCEPT, "application/json")
            .send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(PubDevError::UnexpectedStatus { status }),
            _ => Ok(Some(response.json()?)),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PubDevPackage {
    latest: PubDevVersion,
}

#[derive(Debug, Deserialize)]
struct PubDevVersion {
    pubspec: PubDevPubspec,
}

#[derive(Debug, Deserialize)]
struct PubDevPubspec {
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default, rename = "issue_tracker")]
    issue_tracker: Option<String>,
    #[serde(default)]
    documentation: Option<String>,
}

impl PubDevPackage {
    pub fn candidate_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        let mut seen = BTreeSet::new();

        let pubspec = &self.latest.pubspec;
        for value in [
            pubspec.repository.as_deref(),
            pubspec.homepage.as_deref(),
            pubspec.issue_tracker.as_deref(),
            pubspec.documentation.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_lowercase()) {
                urls.push(trimmed.to_string());
            }
        }

        urls
    }
}

pub struct DartDiscoverer<F: PubDevFetcher> {
    fetcher: F,
}

impl Default for DartDiscoverer<HttpPubDevClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl DartDiscoverer<HttpPubDevClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpPubDevClient::new(),
        }
    }
}

impl<F: PubDevFetcher> DartDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, DartDiscoveryError> {
        let path = project_root.join(PUBSPEC_FILE);
        let content = fs::read_to_string(&path).map_err(|err| DartDiscoveryError::Io {
            path: path.display().to_string(),
            source: err,
        })?;

        let value: Value =
            serde_yaml::from_str(&content).map_err(|err| DartDiscoveryError::Yaml {
                path: path.display().to_string(),
                source: err,
            })?;

        let mut hosted = BTreeSet::new();
        let mut git_urls = BTreeSet::new();

        if let Some(deps) = value.get("dependencies").and_then(Value::as_mapping) {
            collect_dependencies(deps, &mut hosted, &mut git_urls);
        }
        if let Some(deps) = value.get("dev_dependencies").and_then(Value::as_mapping) {
            collect_dependencies(deps, &mut hosted, &mut git_urls);
        }
        if let Some(deps) = value
            .get("dependency_overrides")
            .and_then(Value::as_mapping)
        {
            collect_dependencies(deps, &mut hosted, &mut git_urls);
        }

        let mut repositories = Vec::new();

        for url in git_urls {
            if let Some(mut repository) = parse_github_repository(&url) {
                repository.via = Some(PUBSPEC_FILE.to_string());
                repositories.push(repository);
            }
        }

        for name in hosted {
            let Some(package) =
                self.fetcher
                    .fetch(&name)
                    .map_err(|source| DartDiscoveryError::PubDev {
                        name: name.clone(),
                        source,
                    })?
            else {
                continue;
            };

            for url in package.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(&url) {
                    repository.via = Some(PUBSPEC_FILE.to_string());
                    repositories.push(repository);
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

fn collect_dependencies(
    mapping: &Mapping,
    hosted: &mut BTreeSet<String>,
    git_urls: &mut BTreeSet<String>,
) {
    for (name_value, details) in mapping {
        let Some(name) = name_value.as_str() else {
            continue;
        };

        match details {
            Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
                hosted.insert(name.to_string());
            }
            Value::Mapping(map) => {
                let git_key = Value::from("git");
                if let Some(git) = map.get(&git_key) {
                    if let Some(url) = git_url(git) {
                        git_urls.insert(url.to_string());
                        continue;
                    }
                }
                let sdk_key = Value::from("sdk");
                let path_key = Value::from("path");
                if map.contains_key(&sdk_key) || map.contains_key(&path_key) {
                    continue;
                }
                hosted.insert(name.to_string());
            }
            Value::Sequence(_) => {
                hosted.insert(name.to_string());
            }
            _ => {
                hosted.insert(name.to_string());
            }
        }
    }
}

fn git_url(value: &Value) -> Option<&str> {
    match value {
        Value::String(url) => Some(url.as_str()),
        Value::Mapping(map) => {
            let url_key = Value::from("url");
            map.get(&url_key).and_then(Value::as_str)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;
    use std::fs;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_hosted_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(PUBSPEC_FILE),
            r#"
name: example
version: 1.0.0
dependencies:
  http: ^1.0.0
"#,
        )
        .unwrap();

        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET)
                .path("/api/packages/http")
                .header("accept", "application/json");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "latest": {
                        "pubspec": {
                            "repository": "https://github.com/example/http"
                        }
                    }
                }));
        });

        let fetcher =
            HttpPubDevClient::with_base_url(format!("{}/api/packages", server.base_url()));
        let discoverer = DartDiscoverer::with_fetcher(fetcher);
        let mut repos = discoverer.discover(dir.path()).unwrap();
        mock.assert();

        assert_eq!(repos.len(), 1);
        let repo = repos.remove(0);
        assert_eq!(repo.owner, "example");
        assert_eq!(repo.name, "http");
        assert_eq!(repo.via.as_deref(), Some(PUBSPEC_FILE));
    }

    #[test]
    fn discovers_git_dependencies_without_fetching() {
        struct PanicFetcher;

        impl PubDevFetcher for PanicFetcher {
            fn fetch(&self, _name: &str) -> Result<Option<PubDevPackage>, PubDevError> {
                panic!("fetch should not be called")
            }
        }

        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(PUBSPEC_FILE),
            r#"
name: example
version: 1.0.0
dependencies:
  awesome:
    git:
      url: https://github.com/example/awesome.git
"#,
        )
        .unwrap();

        let discoverer = DartDiscoverer::with_fetcher(PanicFetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        let repo = &repos[0];
        assert_eq!(repo.owner, "example");
        assert_eq!(repo.name, "awesome");
        assert_eq!(repo.via.as_deref(), Some(PUBSPEC_FILE));
    }

    #[test]
    fn includes_dependency_overrides() {
        struct RecordingFetcher {
            names: std::sync::Mutex<Vec<String>>,
        }

        impl RecordingFetcher {
            fn new() -> Self {
                Self {
                    names: std::sync::Mutex::new(Vec::new()),
                }
            }
        }

        impl PubDevFetcher for Arc<RecordingFetcher> {
            fn fetch(&self, name: &str) -> Result<Option<PubDevPackage>, PubDevError> {
                self.names.lock().unwrap().push(name.to_string());
                Ok(Some(PubDevPackage {
                    latest: PubDevVersion {
                        pubspec: PubDevPubspec {
                            repository: Some(format!("https://github.com/example/{name}")),
                            homepage: None,
                            issue_tracker: None,
                            documentation: None,
                        },
                    },
                }))
            }
        }

        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(PUBSPEC_FILE),
            r#"
name: example
version: 1.0.0
dependency_overrides:
  hosted_dep: any
  git_dep:
    git: https://github.com/example/git_dep
  local_dep:
    path: ../local_dep
"#,
        )
        .unwrap();

        let fetcher = Arc::new(RecordingFetcher::new());
        let discoverer = DartDiscoverer::with_fetcher(fetcher.clone());
        let repos = discoverer.discover(dir.path()).unwrap();

        // hosted dependency should trigger a fetch, git should be parsed locally, path ignored
        assert_eq!(fetcher.names.lock().unwrap().as_slice(), &["hosted_dep"]);

        assert_eq!(repos.len(), 2);
        assert!(repos.iter().any(|repo| repo.name == "hosted_dep"));
        assert!(repos.iter().any(|repo| repo.name == "git_dep"));
    }
}
