use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;
use serde_yaml::Value as YamlValue;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum HaskellDiscoveryError {
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
    Hackage {
        name: String,
        #[source]
        source: HackageError,
    },
}

pub trait HackageFetcher {
    fn fetch(&self, name: &str) -> Result<Option<HackagePackage>, HackageError>;
}

#[derive(Clone)]
pub struct HttpHackageClient {
    client: Client,
    base_url: String,
}

impl Default for HttpHackageClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpHackageClient {
    const DEFAULT_BASE_URL: &'static str = "https://hackage.haskell.org/package";

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

impl HackageFetcher for HttpHackageClient {
    fn fetch(&self, name: &str) -> Result<Option<HackagePackage>, HackageError> {
        let url = format!(
            "{}/{name}/{name}.cabal",
            self.base_url.trim_end_matches('/')
        );
        let response = self.client.get(&url).header(ACCEPT, "text/plain").send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(HackageError::UnexpectedStatus { status }),
            _ => {
                let cabal = response.text()?;
                Ok(Some(HackagePackage::from_cabal(&cabal)))
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HackageError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
}

#[derive(Clone, Debug, Default)]
pub struct HackagePackage {
    urls: Vec<String>,
}

impl HackagePackage {
    pub fn candidate_urls(&self) -> Vec<String> {
        self.urls.clone()
    }

    fn from_cabal(cabal: &str) -> Self {
        let mut urls = Vec::new();
        let mut seen = BTreeSet::new();
        let mut in_source_repo = false;

        for line in cabal.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("--") {
                continue;
            }
            if trimmed.is_empty() {
                in_source_repo = false;
                continue;
            }
            if trimmed.starts_with("source-repository ") {
                in_source_repo = true;
                continue;
            }
            if !line.starts_with(' ') && !line.starts_with('\t') {
                in_source_repo = false;
            }
            if let Some(rest) = trimmed.strip_prefix("homepage:") {
                push_url(rest, &mut urls, &mut seen);
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("bug-reports:") {
                push_url(rest, &mut urls, &mut seen);
                continue;
            }
            if in_source_repo {
                if let Some(rest) = trimmed.strip_prefix("location:") {
                    push_url(rest, &mut urls, &mut seen);
                }
            }
        }

        Self { urls }
    }
}

fn push_url(rest: &str, urls: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    let url = rest.trim();
    if url.is_empty() {
        return;
    }
    let canonical = url.to_lowercase();
    if seen.insert(canonical) {
        urls.push(url.to_string());
    }
}

pub struct HaskellDiscoverer<F: HackageFetcher> {
    fetcher: F,
}

impl Default for HaskellDiscoverer<HttpHackageClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl HaskellDiscoverer<HttpHackageClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpHackageClient::new(),
        }
    }
}

impl<F: HackageFetcher> HaskellDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, HaskellDiscoveryError> {
        let mut dependencies: DependencyMap = BTreeMap::new();

        collect_package_yaml_dependencies(project_root, &mut dependencies)?;
        collect_cabal_dependencies(project_root, &mut dependencies)?;

        let mut repositories = Vec::new();
        for (name, vias) in dependencies {
            let Some(package) =
                self.fetcher
                    .fetch(&name)
                    .map_err(|source| HaskellDiscoveryError::Hackage {
                        name: name.clone(),
                        source,
                    })?
            else {
                continue;
            };

            for url in package.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(&url) {
                    if let Some(via) = vias.iter().next() {
                        repository.via = Some(via.clone());
                    } else {
                        repository.via = Some("Hackage".to_string());
                    }
                    repositories.push(repository);
                    break;
                }
            }
        }

        Ok(repositories)
    }
}

type DependencyMap = BTreeMap<String, BTreeSet<String>>;

fn collect_package_yaml_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), HaskellDiscoveryError> {
    let path = project_root.join("package.yaml");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(HaskellDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let value: YamlValue =
        serde_yaml::from_str(&content).map_err(|err| HaskellDiscoveryError::Yaml {
            path: path.display().to_string(),
            source: err,
        })?;

    if let Some(deps) = value.get("dependencies") {
        match deps {
            YamlValue::Sequence(seq) => {
                for entry in seq {
                    match entry {
                        YamlValue::String(value) => {
                            if let Some(name) = parse_dependency_name(value) {
                                add_dependency(dependencies, &name, "package.yaml");
                            }
                        }
                        YamlValue::Mapping(map) => {
                            if let Some(name) = map
                                .get(&YamlValue::from("package"))
                                .and_then(|v| v.as_str())
                                .or_else(|| {
                                    map.get(&YamlValue::from("name")).and_then(|v| v.as_str())
                                })
                            {
                                if let Some(name) = parse_dependency_name(name) {
                                    add_dependency(dependencies, &name, "package.yaml");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            YamlValue::String(value) => {
                if let Some(name) = parse_dependency_name(value) {
                    add_dependency(dependencies, &name, "package.yaml");
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn collect_cabal_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), HaskellDiscoveryError> {
    let entries = match project_root.read_dir() {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(HaskellDiscoveryError::Io {
                path: project_root.display().to_string(),
                source: err,
            })
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                return Err(HaskellDiscoveryError::Io {
                    path: project_root.display().to_string(),
                    source: err,
                })
            }
        };
        let path = entry.path();
        if !is_cabal_file(&path) {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|err| HaskellDiscoveryError::Io {
            path: path.display().to_string(),
            source: err,
        })?;
        let via = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("*.cabal");
        for dep in parse_cabal_dependencies(&content) {
            add_dependency(dependencies, &dep, via);
        }
    }

    Ok(())
}

fn is_cabal_file(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("cabal"))
            .unwrap_or(false)
}

fn parse_cabal_dependencies(content: &str) -> BTreeSet<String> {
    let mut dependencies = BTreeSet::new();
    let mut lines = content.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed.starts_with("--") {
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("build-depends:")
            .or_else(|| trimmed.strip_prefix("build-tool-depends:"))
        {
            let mut buffer = rest.trim().to_string();
            while let Some(next) = lines.peek() {
                let next_trimmed = next.trim();
                if next_trimmed.starts_with("--") {
                    lines.next();
                    continue;
                }
                let is_indented = next.starts_with(' ') || next.starts_with('\t');
                if next_trimmed.starts_with(',') {
                    buffer.push(' ');
                    buffer.push_str(next_trimmed);
                    lines.next();
                    continue;
                }
                if is_indented && !next_trimmed.contains(':') {
                    buffer.push_str(", ");
                    buffer.push_str(next_trimmed);
                    lines.next();
                    continue;
                }
                break;
            }
            extract_dependencies(&buffer, &mut dependencies);
        }
    }
    dependencies
}

fn extract_dependencies(list: &str, dependencies: &mut BTreeSet<String>) {
    for entry in list.split(',') {
        let raw = entry.trim();
        if raw.is_empty() {
            continue;
        }
        let raw = raw.split("--").next().unwrap_or(raw).trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(name) = parse_dependency_name(raw) {
            dependencies.insert(name);
        }
    }
}

fn parse_dependency_name(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    if chars.next()?.is_digit(10) {
        return None;
    }
    let name = trimmed
        .split(|c: char| c.is_whitespace() || c == '(' || c == ':')
        .next()
        .unwrap_or(trimmed)
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn add_dependency(map: &mut DependencyMap, name: &str, via: &str) {
    map.entry(name.to_string())
        .or_default()
        .insert(via.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    struct MockHackageFetcher {
        packages: HashMap<String, Option<HackagePackage>>,
    }

    impl MockHackageFetcher {
        fn new(packages: HashMap<String, Option<HackagePackage>>) -> Self {
            Self { packages }
        }
    }

    impl HackageFetcher for MockHackageFetcher {
        fn fetch(&self, name: &str) -> Result<Option<HackagePackage>, HackageError> {
            Ok(self.packages.get(name).cloned().unwrap_or(None))
        }
    }

    #[test]
    fn parses_hackage_package_urls() {
        let cabal = r#"
name: example
version: 0.1.0.0
homepage: https://github.com/org/project
bug-reports: https://github.com/org/project/issues
source-repository head
  type: git
  location: https://github.com/org/project.git
"#;
        let package = HackagePackage::from_cabal(cabal);
        let urls = package.candidate_urls();
        assert!(urls.contains(&"https://github.com/org/project".to_string()));
        assert!(urls.contains(&"https://github.com/org/project/issues".to_string()));
        assert!(urls.contains(&"https://github.com/org/project.git".to_string()));
    }

    #[test]
    fn discovers_dependencies_from_package_yaml() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.yaml"),
            r#"
dependencies:
  - text >= 1.2
  - package: bytestring
    version: ">=0.11"
"#,
        )
        .unwrap();

        let mut packages = HashMap::new();
        packages.insert(
            "text".to_string(),
            Some(HackagePackage {
                urls: vec!["https://github.com/haskell/text".to_string()],
            }),
        );
        packages.insert(
            "bytestring".to_string(),
            Some(HackagePackage {
                urls: vec!["https://github.com/haskell/bytestring".to_string()],
            }),
        );

        let discoverer = HaskellDiscoverer::with_fetcher(MockHackageFetcher::new(packages));
        let repos = discoverer.discover(dir.path()).unwrap();

        let owners: Vec<_> = repos
            .iter()
            .map(|repo| (repo.owner.as_str(), repo.name.as_str()))
            .collect();
        assert!(owners.contains(&("haskell", "text")));
        assert!(owners.contains(&("haskell", "bytestring")));
        for repo in repos {
            assert_eq!(repo.via.as_deref(), Some("package.yaml"));
        }
    }

    #[test]
    fn discovers_dependencies_from_cabal_file() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("example.cabal"),
            r#"
name: example
version: 0.1.0.0
build-depends: text >= 1.2,
               bytestring -- comment
"#,
        )
        .unwrap();

        let mut packages = HashMap::new();
        packages.insert(
            "text".to_string(),
            Some(HackagePackage {
                urls: vec!["https://github.com/haskell/text".to_string()],
            }),
        );
        packages.insert(
            "bytestring".to_string(),
            Some(HackagePackage {
                urls: vec!["https://github.com/haskell/bytestring".to_string()],
            }),
        );

        let discoverer = HaskellDiscoverer::with_fetcher(MockHackageFetcher::new(packages));
        let repos = discoverer.discover(dir.path()).unwrap();

        let owners: Vec<_> = repos
            .iter()
            .map(|repo| (repo.owner.as_str(), repo.name.as_str(), repo.via.as_deref()))
            .collect();
        assert!(owners.contains(&("haskell", "text", Some("example.cabal"))));
        assert!(owners.contains(&("haskell", "bytestring", Some("example.cabal"))));
    }
}
