use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum RubyDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to fetch metadata for gem {name}: {source}")]
    RubyGems {
        name: String,
        #[source]
        source: RubyGemsError,
    },
}

pub trait RubyGemsFetcher {
    fn fetch(&self, name: &str) -> Result<Option<RubyGem>, RubyGemsError>;
}

#[derive(Clone)]
pub struct HttpRubyGemsClient {
    client: Client,
    base_url: String,
}

impl Default for HttpRubyGemsClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpRubyGemsClient {
    const DEFAULT_BASE_URL: &'static str = "https://rubygems.org/api/v1/gems";

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

impl RubyGemsFetcher for HttpRubyGemsClient {
    fn fetch(&self, name: &str) -> Result<Option<RubyGem>, RubyGemsError> {
        let url = format!("{}/{name}.json", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .header(ACCEPT, "application/json")
            .send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(RubyGemsError::UnexpectedStatus { status }),
            _ => Ok(Some(response.json()?)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RubyGemsError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
}

pub struct RubyDiscoverer<F: RubyGemsFetcher> {
    fetcher: F,
}

impl Default for RubyDiscoverer<HttpRubyGemsClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl RubyDiscoverer<HttpRubyGemsClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpRubyGemsClient::new(),
        }
    }
}

impl<F: RubyGemsFetcher> RubyDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, RubyDiscoveryError> {
        let mut names = BTreeSet::new();
        for name in read_gemfile_lock(project_root)? {
            names.insert(name);
        }
        for name in read_gemfile(project_root)? {
            names.insert(name);
        }

        let mut repositories = Vec::new();
        for name in names {
            let Some(gem) =
                self.fetcher
                    .fetch(&name)
                    .map_err(|source| RubyDiscoveryError::RubyGems {
                        name: name.clone(),
                        source,
                    })?
            else {
                continue;
            };

            for candidate in gem.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(candidate) {
                    repository.via = Some("RubyGems".to_string());
                    repositories.push(repository);
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

fn read_gemfile_lock(project_root: &Path) -> Result<Vec<String>, RubyDiscoveryError> {
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

    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            if in_dependencies {
                break;
            }
            continue;
        }
        if line.starts_with("DEPENDENCIES") {
            in_dependencies = true;
            continue;
        }
        if in_dependencies {
            if !line.starts_with(' ') && !line.starts_with('\t') {
                break;
            }
            if let Some(name) = line.split_whitespace().next() {
                if let Some(normalized) = normalize_dependency_name(name) {
                    names.push(normalized);
                }
            }
        }
    }

    Ok(names)
}

fn read_gemfile(project_root: &Path) -> Result<Vec<String>, RubyDiscoveryError> {
    let gemfile_path = project_root.join("Gemfile");
    let content = match fs::read_to_string(&gemfile_path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(RubyDiscoveryError::Io {
                path: gemfile_path.display().to_string(),
                source: err,
            })
        }
    };

    let regex = Regex::new(r#"(?m)^\s*gem\s+['"]([^'"]+)['"]"#).unwrap();
    let mut names = Vec::new();
    for caps in regex.captures_iter(&content) {
        if let Some(name) = caps
            .get(1)
            .and_then(|capture| normalize_dependency_name(capture.as_str()))
        {
            names.push(name);
        }
    }

    Ok(names)
}

fn normalize_dependency_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let normalized = trimmed.trim_end_matches('!').trim();
    if normalized.is_empty() {
        return None;
    }

    Some(normalized.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RubyGem {
    #[serde(default)]
    source_code_uri: Option<String>,
    #[serde(default)]
    homepage_uri: Option<String>,
    #[serde(default)]
    wiki_uri: Option<String>,
    #[serde(default)]
    documentation_uri: Option<String>,
    #[serde(default)]
    bug_tracker_uri: Option<String>,
    #[serde(default)]
    metadata: Option<RubyGemMetadata>,
}

impl RubyGem {
    fn candidate_urls(&self) -> Vec<&str> {
        let mut urls = Vec::new();
        push_url(&mut urls, self.source_code_uri.as_deref());
        push_url(&mut urls, self.homepage_uri.as_deref());
        push_url(&mut urls, self.bug_tracker_uri.as_deref());
        push_url(&mut urls, self.documentation_uri.as_deref());
        push_url(&mut urls, self.wiki_uri.as_deref());
        if let Some(metadata) = &self.metadata {
            metadata.extend_urls(&mut urls);
        }
        urls
    }
}

#[derive(Debug, Deserialize)]
pub struct RubyGemMetadata {
    #[serde(default)]
    source_code_uri: Option<String>,
    #[serde(default)]
    homepage_uri: Option<String>,
    #[serde(default)]
    wiki_uri: Option<String>,
    #[serde(default)]
    documentation_uri: Option<String>,
    #[serde(default)]
    bug_tracker_uri: Option<String>,
}

impl RubyGemMetadata {
    fn extend_urls<'a>(&'a self, target: &mut Vec<&'a str>) {
        push_url(target, self.source_code_uri.as_deref());
        push_url(target, self.homepage_uri.as_deref());
        push_url(target, self.bug_tracker_uri.as_deref());
        push_url(target, self.documentation_uri.as_deref());
        push_url(target, self.wiki_uri.as_deref());
    }
}

fn push_url<'a>(target: &mut Vec<&'a str>, candidate: Option<&'a str>) {
    if let Some(url) = candidate {
        if !url.trim().is_empty() {
            target.push(url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tempfile::tempdir;

    struct StubFetcher {
        responses: RefCell<HashMap<String, Option<RubyGem>>>,
    }

    impl StubFetcher {
        fn new(responses: impl IntoIterator<Item = (String, Option<RubyGem>)>) -> Self {
            Self {
                responses: RefCell::new(responses.into_iter().collect()),
            }
        }
    }

    impl RubyGemsFetcher for StubFetcher {
        fn fetch(&self, name: &str) -> Result<Option<RubyGem>, RubyGemsError> {
            Ok(self.responses.borrow_mut().remove(name).unwrap_or(None))
        }
    }

    #[test]
    fn discovers_repositories_from_gemfile_lock() {
        let dir = tempdir().unwrap();
        let lock_contents = r#"GEM
  remote: https://rubygems.org/
  specs:
    rack (2.2.3)

DEPENDENCIES
  rack (= 2.2.3)
"#;
        fs::write(dir.path().join("Gemfile.lock"), lock_contents).unwrap();

        let fetcher = StubFetcher::new(vec![(
            "rack".to_string(),
            Some(RubyGem {
                source_code_uri: Some("https://github.com/rack/rack".to_string()),
                homepage_uri: None,
                wiki_uri: None,
                documentation_uri: None,
                bug_tracker_uri: None,
                metadata: None,
            }),
        )]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "rack");
        assert_eq!(repos[0].name, "rack");
        assert_eq!(repos[0].via.as_deref(), Some("RubyGems"));
    }

    #[test]
    fn discovers_repositories_from_gemfile_when_lock_missing() {
        let dir = tempdir().unwrap();
        let gemfile = r#"source 'https://rubygems.org'

gem "rails"
"#;
        fs::write(dir.path().join("Gemfile"), gemfile).unwrap();

        let fetcher = StubFetcher::new(vec![(
            "rails".to_string(),
            Some(RubyGem {
                source_code_uri: None,
                homepage_uri: Some("https://github.com/rails/rails".to_string()),
                wiki_uri: None,
                documentation_uri: None,
                bug_tracker_uri: None,
                metadata: None,
            }),
        )]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "rails");
        assert_eq!(repos[0].name, "rails");
    }

    #[test]
    fn skips_dependencies_when_no_metadata_found() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Gemfile"), "gem 'unknown'").unwrap();

        let fetcher = StubFetcher::new(vec![("unknown".to_string(), None)]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }

    #[test]
    fn uses_metadata_fallback_urls() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Gemfile"), "gem 'rspec'").unwrap();

        let fetcher = StubFetcher::new(vec![(
            "rspec".to_string(),
            Some(RubyGem {
                source_code_uri: None,
                homepage_uri: None,
                wiki_uri: None,
                documentation_uri: None,
                bug_tracker_uri: None,
                metadata: Some(RubyGemMetadata {
                    source_code_uri: Some("https://github.com/rspec/rspec".to_string()),
                    homepage_uri: None,
                    wiki_uri: None,
                    documentation_uri: None,
                    bug_tracker_uri: None,
                }),
            }),
        )]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "rspec");
    }

    #[test]
    fn ignores_empty_and_duplicate_dependency_names() {
        let dir = tempdir().unwrap();
        let gemfile = r#"gem "rails"
        gem "rails"
        gem ""
"#;
        fs::write(dir.path().join("Gemfile"), gemfile).unwrap();

        let fetcher = StubFetcher::new(vec![(
            "rails".to_string(),
            Some(RubyGem {
                source_code_uri: Some("https://github.com/rails/rails".to_string()),
                homepage_uri: None,
                wiki_uri: None,
                documentation_uri: None,
                bug_tracker_uri: None,
                metadata: None,
            }),
        )]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();
        assert_eq!(repos.len(), 1);
    }

    #[test]
    fn normalizes_git_dependencies_in_lockfile() {
        let dir = tempdir().unwrap();
        let lock_contents = r#"GIT
  remote: https://github.com/sparklemotion/nokogiri.git
  revision: deadbeef
  specs:
    nokogiri (1.16.5)

DEPENDENCIES
  nokogiri!
"#;
        fs::write(dir.path().join("Gemfile.lock"), lock_contents).unwrap();

        let fetcher = StubFetcher::new(vec![(
            "nokogiri".to_string(),
            Some(RubyGem {
                source_code_uri: Some("https://github.com/sparklemotion/nokogiri".to_string()),
                homepage_uri: None,
                wiki_uri: None,
                documentation_uri: None,
                bug_tracker_uri: None,
                metadata: None,
            }),
        )]);

        let discoverer = RubyDiscoverer::with_fetcher(fetcher);
        let repos = discoverer.discover(dir.path()).unwrap();

        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "nokogiri");
    }

    #[test]
    fn normalize_dependency_name_handles_edge_cases() {
        assert_eq!(normalize_dependency_name("arel!"), Some("arel".to_string()));
        assert_eq!(
            normalize_dependency_name("  foo  "),
            Some("foo".to_string())
        );
        assert!(normalize_dependency_name("   ").is_none());
    }
}
