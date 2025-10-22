use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

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
}

#[derive(Default)]
pub struct PythonUvDiscoverer;

#[derive(Default)]
pub struct PythonPipDiscoverer;

impl PythonUvDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, PythonDiscoveryError> {
        let lock_path = project_root.join("uv.lock");
        let content = fs::read_to_string(&lock_path).map_err(|err| PythonDiscoveryError::Io {
            path: lock_path.display().to_string(),
            source: err,
        })?;

        let lock: UvLock = toml::from_str(&content).map_err(|err| PythonDiscoveryError::Toml {
            path: lock_path.display().to_string(),
            source: err,
        })?;

        let mut packages = BTreeSet::new();
        for package in lock.package.unwrap_or_default() {
            if package
                .source
                .as_ref()
                .map(|source| source.registry.is_some())
                .unwrap_or(false)
            {
                packages.insert(package.name);
            }
        }

        Ok(collect_repositories(project_root, &packages, "uv.lock"))
    }
}

impl PythonPipDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, PythonDiscoveryError> {
        let requirements_path = project_root.join("requirements.txt");
        let content =
            fs::read_to_string(&requirements_path).map_err(|err| PythonDiscoveryError::Io {
                path: requirements_path.display().to_string(),
                source: err,
            })?;

        let mut packages = BTreeSet::new();
        for line in content.lines() {
            if let Some(name) = parse_requirement_line(line) {
                packages.insert(name);
            }
        }

        Ok(collect_repositories(
            project_root,
            &packages,
            "requirements.txt",
        ))
    }
}

fn collect_repositories(
    project_root: &Path,
    packages: &BTreeSet<String>,
    via: &str,
) -> Vec<Repository> {
    let envs = find_python_environments(project_root);
    let site_packages = envs
        .iter()
        .flat_map(|env| find_site_packages(env.as_path()))
        .collect::<Vec<_>>();

    let mut repositories = Vec::new();
    let mut seen = BTreeSet::new();

    for package in packages {
        if seen.contains(package) {
            continue;
        }
        if let Some(mut repo) = repository_from_site_packages(&site_packages, package) {
            repo.via = Some(via.to_string());
            repositories.push(repo);
            seen.insert(package.clone());
        }
    }

    repositories
}

fn repository_from_site_packages(site_packages: &[PathBuf], package: &str) -> Option<Repository> {
    for base in site_packages {
        if let Some(repo) = repository_from_site_package(base, package) {
            return Some(repo);
        }
    }
    None
}

fn repository_from_site_package(base: &Path, package: &str) -> Option<Repository> {
    let normalized = normalize_package_name(package);
    let entries = fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = path.file_name()?.to_string_lossy().to_string();
        if file_name.ends_with(".dist-info") {
            let stem = file_name.trim_end_matches(".dist-info");
            let normalized_stem = normalize_package_name(stem);
            if normalized_stem == normalized
                || normalized_stem.starts_with(&(normalized.clone() + "-"))
            {
                let metadata_path = path.join("METADATA");
                if let Ok(content) = fs::read_to_string(&metadata_path) {
                    if let Some(repo) = repository_from_metadata(&content) {
                        return Some(repo);
                    }
                }
            }
        } else if file_name.ends_with(".egg-info") {
            let stem = file_name.trim_end_matches(".egg-info");
            let normalized_stem = normalize_package_name(stem);
            if normalized_stem == normalized
                || normalized_stem.starts_with(&(normalized.clone() + "-"))
            {
                let metadata_path = path.join("PKG-INFO");
                if let Ok(content) = fs::read_to_string(&metadata_path) {
                    if let Some(repo) = repository_from_metadata(&content) {
                        return Some(repo);
                    }
                }
            }
        }
    }
    None
}

fn repository_from_metadata(metadata: &str) -> Option<Repository> {
    for line in metadata.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Home-page:") || trimmed.starts_with("Project-URL:") {
            if let Some(idx) = trimmed.find("http") {
                let url = trimmed[idx..].trim();
                if let Some(repo) = parse_github_repository(url) {
                    return Some(repo);
                }
            }
        }
    }
    None
}

fn parse_requirement_line(line: &str) -> Option<String> {
    let line = line.split('#').next()?.trim();
    if line.is_empty() {
        return None;
    }
    let token = line.split_whitespace().next()?;
    let mut end = token.len();
    for ch in ['[', '=', '<', '>', '!', '~', ';', '@'] {
        if let Some(idx) = token.find(ch) {
            end = end.min(idx);
        }
    }
    let name = token[..end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn normalize_package_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

fn find_python_environments(project_root: &Path) -> Vec<PathBuf> {
    let mut envs = BTreeSet::new();
    for candidate in [".venv", "venv"] {
        let path = project_root.join(candidate);
        if path.is_dir() {
            envs.insert(path);
        }
    }

    if let Ok(entries) = fs::read_dir(project_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && path.join("pyvenv.cfg").exists() {
                envs.insert(path);
            }
        }
    }

    envs.into_iter().collect()
}

fn find_site_packages(env: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let lib = env.join("lib");
    if let Ok(entries) = fs::read_dir(&lib) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("python") {
                    let site = path.join("site-packages");
                    if site.is_dir() {
                        paths.push(site);
                    }
                }
            }
        }
    }

    let windows_site = env.join("Lib/site-packages");
    if windows_site.is_dir() {
        paths.push(windows_site);
    }

    paths
}

#[derive(Deserialize)]
struct UvLock {
    #[serde(default)]
    package: Option<Vec<UvPackage>>,
}

#[derive(Deserialize)]
struct UvPackage {
    name: String,
    #[serde(default)]
    source: Option<UvSource>,
}

#[derive(Deserialize, Default)]
struct UvSource {
    #[serde(default)]
    registry: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn create_metadata(env: &Path, name: &str, version: &str, metadata: &str) {
        let site = env.join("lib/python3.12/site-packages");
        fs::create_dir_all(&site).unwrap();
        let dist_info = site.join(format!("{name}-{version}.dist-info"));
        fs::create_dir_all(&dist_info).unwrap();
        fs::write(dist_info.join("METADATA"), metadata).unwrap();
    }

    #[test]
    fn discovers_repositories_from_uv_lock() {
        let dir = tempdir().unwrap();
        let project = dir.path();
        fs::write(project.join("uv.lock"), "version = 1\n[[package]]\nname = \"dep-one\"\nversion = \"1.0.0\"\nsource = { registry = \"https://pypi.org/simple\" }\n[[package]]\nname = \"project\"\nversion = \"0.1.0\"\nsource = { virtual = \".\" }\n").unwrap();

        let env = project.join(".venv");
        fs::create_dir_all(&env).unwrap();
        create_metadata(
            &env,
            "dep-one",
            "1.0.0",
            "Metadata-Version: 2.1\nHome-page: https://github.com/example/dep-one\n",
        );

        let discoverer = PythonUvDiscoverer::new();
        let repos = discoverer.discover(project).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "dep-one");
    }

    #[test]
    fn discovers_repositories_from_requirements() {
        let dir = tempdir().unwrap();
        let project = dir.path();
        fs::write(
            project.join("requirements.txt"),
            "requests==1.0.0\n# comment\n",
        )
        .unwrap();

        let env = project.join("venv");
        fs::create_dir_all(&env).unwrap();
        create_metadata(
            &env,
            "requests",
            "1.0.0",
            "Metadata-Version: 2.1\nProject-URL: Source, https://github.com/example/requests\n",
        );

        let discoverer = PythonPipDiscoverer::new();
        let repos = discoverer.discover(project).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].owner, "example");
        assert_eq!(repos[0].name, "requests");
    }
}
