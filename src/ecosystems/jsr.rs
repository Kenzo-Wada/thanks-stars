use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum JsrDiscoveryError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path} as JSON: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Default)]
pub struct JsrDiscoverer;

impl JsrDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, JsrDiscoveryError> {
        let manifest_path = project_root.join("jsr.json");
        let manifest_content = match fs::read_to_string(&manifest_path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(err) => {
                return Err(JsrDiscoveryError::Io {
                    path: manifest_path.display().to_string(),
                    source: err,
                });
            }
        };

        let manifest: Value =
            serde_json::from_str(&manifest_content).map_err(|err| JsrDiscoveryError::Json {
                path: manifest_path.display().to_string(),
                source: err,
            })?;

        let mut names = BTreeSet::new();
        add_dependency_names(&mut names, &manifest, "dependencies");
        add_dependency_names(&mut names, &manifest, "devDependencies");

        let mut repositories = Vec::new();
        let mut seen = BTreeSet::new();

        for name in names {
            if let Some(mut repo) = repository_from_package(project_root, &name)? {
                if seen.insert((repo.owner.clone(), repo.name.clone())) {
                    repo.via = Some("jsr.json".to_string());
                    repositories.push(repo);
                }
            }
        }

        Ok(repositories)
    }
}

fn add_dependency_names(target: &mut BTreeSet<String>, manifest: &Value, key: &str) {
    if let Some(deps) = manifest.get(key).and_then(|value| value.as_object()) {
        for name in deps.keys() {
            target.insert(name.to_string());
        }
    }
}

fn repository_from_package(
    project_root: &Path,
    name: &str,
) -> Result<Option<Repository>, JsrDiscoveryError> {
    let paths = package_metadata_paths(project_root, name);
    for path in paths {
        if let Ok(content) = fs::read_to_string(&path) {
            let json: Value =
                serde_json::from_str(&content).map_err(|err| JsrDiscoveryError::Json {
                    path: path.display().to_string(),
                    source: err,
                })?;
            if let Some(repo) = repository_from_manifest(&json) {
                if let Some(mut repository) = parse_github_repository(&repo) {
                    repository.via = None;
                    return Ok(Some(repository));
                }
            }
        }
    }
    Ok(None)
}

fn package_metadata_paths(project_root: &Path, name: &str) -> Vec<PathBuf> {
    let mut base = project_root.join(".jsr");
    for segment in name.split('/') {
        base.push(segment);
    }

    vec![base.join("package.json")]
}

fn repository_from_manifest(manifest: &Value) -> Option<String> {
    if let Some(repo) = manifest.get("repository") {
        match repo {
            Value::String(value) => return Some(value.clone()),
            Value::Object(map) => {
                if let Some(Value::String(url)) = map.get("url") {
                    return Some(url.clone());
                }
            }
            _ => {}
        }
    }

    if let Some(Value::String(homepage)) = manifest.get("homepage") {
        return Some(homepage.clone());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_jsr_packages() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("jsr.json"),
            json!({ "dependencies": { "@scope/pkg": "1.0.0", "plain": "^2.0.0" } }).to_string(),
        )
        .unwrap();

        let scoped_dir = dir.path().join(".jsr/@scope/pkg");
        fs::create_dir_all(&scoped_dir).unwrap();
        fs::write(
            scoped_dir.join("package.json"),
            json!({ "repository": { "url": "https://github.com/scope/pkg" } }).to_string(),
        )
        .unwrap();

        let plain_dir = dir.path().join(".jsr/plain");
        fs::create_dir_all(&plain_dir).unwrap();
        fs::write(
            plain_dir.join("package.json"),
            json!({ "homepage": "https://github.com/plain/project" }).to_string(),
        )
        .unwrap();

        let discoverer = JsrDiscoverer::new();
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "pkg");
        assert_eq!(repos[0].via.as_deref(), Some("jsr.json"));
        assert_eq!(repos[1].name, "project");
    }

    #[test]
    fn returns_empty_when_manifest_missing() {
        let dir = tempdir().unwrap();
        let discoverer = JsrDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }
}
