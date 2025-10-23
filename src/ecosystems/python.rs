use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum PythonDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as TOML: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to fetch metadata for package {name}: {source}")]
    PyPi {
        name: String,
        #[source]
        source: PyPiError,
    },
}

/// Abstraction over the [PyPI JSON API](https://warehouse.pypa.io/api-reference/json.html)
/// used to look up repository metadata for packages discovered in Python
/// manifests.
pub trait PyPiFetcher {
    fn fetch(&self, name: &str) -> Result<Option<PyPiProject>, PyPiError>;
}

/// Thin wrapper around [`reqwest`] that talks to the live PyPI service.
#[derive(Clone)]
pub struct HttpPyPiClient {
    client: Client,
    base_url: String,
}

impl Default for HttpPyPiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpPyPiClient {
    const DEFAULT_BASE_URL: &'static str = "https://pypi.org/pypi";

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

impl PyPiFetcher for HttpPyPiClient {
    fn fetch(&self, name: &str) -> Result<Option<PyPiProject>, PyPiError> {
        let url = format!("{}/{name}/json", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .header(ACCEPT, "application/json")
            .send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(PyPiError::UnexpectedStatus { status }),
            _ => Ok(Some(response.json()?)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PyPiError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
}

#[derive(Clone, Debug, Deserialize)]
pub struct PyPiProject {
    info: PyPiInfo,
}

#[derive(Clone, Debug, Deserialize)]
struct PyPiInfo {
    #[serde(default)]
    home_page: Option<String>,
    #[serde(default)]
    project_urls: Option<BTreeMap<String, String>>,
}

impl PyPiProject {
    pub fn candidate_urls(&self) -> Vec<String> {
        let mut urls = Vec::new();
        let mut seen = BTreeSet::new();

        if let Some(map) = &self.info.project_urls {
            const PRIORITY_KEYS: [&str; 4] = ["Source", "Homepage", "Code", "Repository"];
            for key in PRIORITY_KEYS {
                if let Some(value) = map.get(key) {
                    let value = value.trim();
                    if !value.is_empty() && seen.insert(value.to_lowercase()) {
                        urls.push(value.to_string());
                    }
                }
            }
            for value in map.values() {
                let value = value.trim();
                if !value.is_empty() && seen.insert(value.to_lowercase()) {
                    urls.push(value.to_string());
                }
            }
        }

        if let Some(home) = &self.info.home_page {
            let home = home.trim();
            if !home.is_empty() && seen.insert(home.to_lowercase()) {
                urls.push(home.to_string());
            }
        }

        urls
    }
}

pub struct PythonDiscoverer<F: PyPiFetcher> {
    fetcher: F,
}

impl Default for PythonDiscoverer<HttpPyPiClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl PythonDiscoverer<HttpPyPiClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpPyPiClient::new(),
        }
    }
}

impl<F: PyPiFetcher> PythonDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, PythonDiscoveryError> {
        let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        collect_pyproject_dependencies(project_root, &mut dependencies)?;
        collect_pipfile_dependencies(project_root, &mut dependencies)?;
        collect_pipfile_lock_dependencies(project_root, &mut dependencies)?;
        collect_requirements_dependencies(project_root, &mut dependencies)?;
        collect_uv_lock_dependencies(project_root, &mut dependencies)?;

        let mut repositories = Vec::new();
        for (name, vias) in dependencies {
            let Some(project) =
                self.fetcher
                    .fetch(&name)
                    .map_err(|source| PythonDiscoveryError::PyPi {
                        name: name.clone(),
                        source,
                    })?
            else {
                continue;
            };

            for url in project.candidate_urls() {
                if let Some(mut repository) = parse_github_repository(&url) {
                    if let Some(via) = vias.iter().next() {
                        repository.via = Some(via.clone());
                    } else {
                        repository.via = Some("PyPI".to_string());
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

fn collect_pyproject_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), PythonDiscoveryError> {
    let path = project_root.join("pyproject.toml");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PythonDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let value: TomlValue = toml::from_str(&content).map_err(|err| PythonDiscoveryError::Toml {
        path: path.display().to_string(),
        source: err,
    })?;

    if let Some(project) = value.get("project") {
        if let Some(array) = project.get("dependencies").and_then(|v| v.as_array()) {
            for entry in array {
                if let Some(dep) = entry.as_str() {
                    add_requirement_dependency(dependencies, dep, "pyproject.toml");
                }
            }
        }
        if let Some(optional) = project
            .get("optional-dependencies")
            .and_then(|v| v.as_table())
        {
            for deps in optional.values() {
                if let Some(array) = deps.as_array() {
                    for entry in array {
                        if let Some(dep) = entry.as_str() {
                            add_requirement_dependency(dependencies, dep, "pyproject.toml");
                        }
                    }
                }
            }
        }
    }

    if let Some(tool) = value.get("tool").and_then(|v| v.as_table()) {
        if let Some(poetry) = tool.get("poetry").and_then(|v| v.as_table()) {
            if let Some(table) = poetry.get("dependencies").and_then(|v| v.as_table()) {
                collect_poetry_table(table, dependencies);
            }
            if let Some(table) = poetry.get("dev-dependencies").and_then(|v| v.as_table()) {
                collect_poetry_table(table, dependencies);
            }
            if let Some(group) = poetry.get("group").and_then(|v| v.as_table()) {
                for value in group.values() {
                    if let Some(table) = value
                        .as_table()
                        .and_then(|table| table.get("dependencies"))
                        .and_then(|v| v.as_table())
                    {
                        collect_poetry_table(table, dependencies);
                    }
                }
            }
        }

        if let Some(uv) = tool.get("uv").and_then(|v| v.as_table()) {
            if let Some(workspace) = uv.get("workspace").and_then(|v| v.as_table()) {
                if let Some(array) = workspace.get("dependencies").and_then(|v| v.as_array()) {
                    for entry in array {
                        if let Some(dep) = entry.as_str() {
                            add_requirement_dependency(dependencies, dep, "pyproject.toml");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn collect_poetry_table(table: &toml::value::Table, dependencies: &mut DependencyMap) {
    for (name, _value) in table {
        if name.eq_ignore_ascii_case("python") {
            continue;
        }
        add_named_dependency(dependencies, name, "pyproject.toml");
    }
}

fn collect_pipfile_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), PythonDiscoveryError> {
    let path = project_root.join("Pipfile");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PythonDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let value: TomlValue = toml::from_str(&content).map_err(|err| PythonDiscoveryError::Toml {
        path: path.display().to_string(),
        source: err,
    })?;

    if let Some(table) = value.get("packages").and_then(|v| v.as_table()) {
        collect_pipfile_table(table, dependencies, "Pipfile");
    }
    if let Some(table) = value.get("dev-packages").and_then(|v| v.as_table()) {
        collect_pipfile_table(table, dependencies, "Pipfile");
    }

    Ok(())
}

fn collect_pipfile_table(table: &toml::value::Table, dependencies: &mut DependencyMap, via: &str) {
    for (name, _value) in table {
        add_named_dependency(dependencies, name, via);
    }
}

fn collect_pipfile_lock_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), PythonDiscoveryError> {
    let path = project_root.join("Pipfile.lock");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PythonDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let value: JsonValue =
        serde_json::from_str(&content).map_err(|err| PythonDiscoveryError::Json {
            path: path.display().to_string(),
            source: err,
        })?;

    for key in ["default", "develop"] {
        if let Some(table) = value.get(key).and_then(|v| v.as_object()) {
            for name in table.keys() {
                add_named_dependency(dependencies, name, "Pipfile.lock");
            }
        }
    }

    Ok(())
}

fn collect_requirements_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), PythonDiscoveryError> {
    let path = project_root.join("requirements.txt");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PythonDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    for line in content.lines() {
        if let Some(name) = normalize_requirement(line) {
            add_dependency(dependencies, name, "requirements.txt");
        }
    }

    Ok(())
}

fn collect_uv_lock_dependencies(
    project_root: &Path,
    dependencies: &mut DependencyMap,
) -> Result<(), PythonDiscoveryError> {
    let path = project_root.join("uv.lock");
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(PythonDiscoveryError::Io {
                path: path.display().to_string(),
                source: err,
            })
        }
    };

    let value: TomlValue = toml::from_str(&content).map_err(|err| PythonDiscoveryError::Toml {
        path: path.display().to_string(),
        source: err,
    })?;

    if let Some(packages) = value.get("package").and_then(|v| v.as_array()) {
        for package in packages {
            if let Some(name) = package
                .as_table()
                .and_then(|table| table.get("name"))
                .and_then(|v| v.as_str())
            {
                add_named_dependency(dependencies, name, "uv.lock");
            }
        }
    }

    Ok(())
}

fn add_dependency(map: &mut DependencyMap, name: String, via: &str) {
    map.entry(name)
        .or_insert_with(BTreeSet::new)
        .insert(via.to_string());
}

fn add_named_dependency(map: &mut DependencyMap, name: &str, via: &str) {
    if let Some(normalized) = normalize_name(name) {
        add_dependency(map, normalized, via);
    }
}

fn add_requirement_dependency(map: &mut DependencyMap, requirement: &str, via: &str) {
    if let Some(normalized) = normalize_requirement(requirement) {
        add_dependency(map, normalized, via);
    }
}

fn normalize_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.replace('_', "-").to_lowercase())
}

fn normalize_requirement(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    if trimmed.starts_with('-') {
        return None;
    }

    let without_marker = trimmed.split(';').next().unwrap_or(trimmed);
    if let Some(egg_index) = without_marker.find("#egg=") {
        let name = &without_marker[egg_index + 5..];
        return normalize_name(name);
    }
    if without_marker.contains("://") {
        return None;
    }

    let mut name = String::new();
    for ch in without_marker.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => name.push(ch),
            '[' | ' ' | '=' | '<' | '>' | '!' | '~' | '(' | ')' | ',' | '@' => break,
            _ => break,
        }
    }

    if name.is_empty() {
        None
    } else {
        normalize_name(&name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use tempfile::tempdir;

    #[derive(Clone)]
    struct StaticPyPiFetcher {
        packages: HashMap<String, Option<PyPiProject>>,
    }

    impl PyPiFetcher for StaticPyPiFetcher {
        fn fetch(&self, name: &str) -> Result<Option<PyPiProject>, PyPiError> {
            Ok(self.packages.get(name).cloned().unwrap_or(None))
        }
    }

    fn project_with_url(url: &str) -> PyPiProject {
        PyPiProject {
            info: PyPiInfo {
                home_page: None,
                project_urls: Some(BTreeMap::from([(String::from("Source"), url.to_string())])),
            },
        }
    }

    #[test]
    fn discovers_repositories_from_python_manifests() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            r#"
[project]
dependencies = ["Requests>=2.0"]
[project.optional-dependencies]
dev = ["httpx==0.27"]
[tool.poetry.dependencies]
python = "^3.11"
numpy = "^1.26"
[tool.poetry.group.dev.dependencies]
pytest = "^7.0"
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("Pipfile"),
            r#"
[packages]
fastapi = "*"
[dev-packages]
ruff = "*"
"#,
        )
        .unwrap();

        fs::write(
            dir.path().join("Pipfile.lock"),
            json!({
                "default": { "starlette": { "version": "==0.37" } },
                "develop": { "mypy": { "version": "==1.8" } }
            })
            .to_string(),
        )
        .unwrap();

        fs::write(
            dir.path().join("requirements.txt"),
            "requests>=2.0\nuvicorn[standard]==0.30\n",
        )
        .unwrap();

        fs::write(
            dir.path().join("uv.lock"),
            r#"
version = 1

[[package]]
name = "httpcore"

[[package]]
name = "uvicorn"
"#,
        )
        .unwrap();

        let fetcher = StaticPyPiFetcher {
            packages: HashMap::from([
                (
                    "requests".to_string(),
                    Some(project_with_url("https://github.com/psf/requests")),
                ),
                (
                    "httpx".to_string(),
                    Some(project_with_url("https://github.com/encode/httpx")),
                ),
                (
                    "numpy".to_string(),
                    Some(project_with_url("https://github.com/numpy/numpy")),
                ),
                (
                    "pytest".to_string(),
                    Some(project_with_url("https://github.com/pytest-dev/pytest")),
                ),
                (
                    "fastapi".to_string(),
                    Some(project_with_url("https://github.com/tiangolo/fastapi")),
                ),
                (
                    "ruff".to_string(),
                    Some(project_with_url("https://github.com/astral-sh/ruff")),
                ),
                (
                    "starlette".to_string(),
                    Some(project_with_url("https://github.com/encode/starlette")),
                ),
                (
                    "mypy".to_string(),
                    Some(project_with_url("https://github.com/python/mypy")),
                ),
                (
                    "uvicorn".to_string(),
                    Some(project_with_url("https://github.com/encode/uvicorn")),
                ),
                (
                    "httpcore".to_string(),
                    Some(project_with_url("https://github.com/encode/httpcore")),
                ),
            ]),
        };

        let discoverer = PythonDiscoverer::with_fetcher(fetcher);
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 10);
        let requests = repos.iter().find(|repo| repo.name == "requests").unwrap();
        assert_eq!(requests.via.as_deref(), Some("pyproject.toml"));
        let uvicorn = repos.iter().find(|repo| repo.name == "uvicorn").unwrap();
        assert_eq!(uvicorn.via.as_deref(), Some("requirements.txt"));
        let fastapi = repos.iter().find(|repo| repo.name == "fastapi").unwrap();
        assert_eq!(fastapi.via.as_deref(), Some("Pipfile"));
        let starlette = repos.iter().find(|repo| repo.name == "starlette").unwrap();
        assert_eq!(starlette.via.as_deref(), Some("Pipfile.lock"));
        let httpcore = repos.iter().find(|repo| repo.name == "httpcore").unwrap();
        assert_eq!(httpcore.via.as_deref(), Some("uv.lock"));
    }

    #[test]
    fn normalize_requirement_parses_basic_specs() {
        assert_eq!(
            normalize_requirement("requests>=2"),
            Some("requests".into())
        );
        assert_eq!(normalize_requirement("numpy"), Some("numpy".into()));
        assert_eq!(
            normalize_requirement("uvicorn[standard]==0.30"),
            Some("uvicorn".into())
        );
        assert_eq!(
            normalize_requirement("git+https://github.com/org/pkg#egg=pkg"),
            Some("pkg".into())
        );
        assert_eq!(normalize_requirement(""), None);
        assert_eq!(normalize_requirement("# comment"), None);
        assert_eq!(normalize_requirement("-r other.txt"), None);
        assert_eq!(normalize_requirement("https://example.com/pkg.whl"), None);
    }
}
