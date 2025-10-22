use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum NodeDiscoveryError {
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
pub struct NodeDiscoverer;

impl NodeDiscoverer {
    pub fn new() -> Self {
        Self
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, NodeDiscoveryError> {
        let package_json_path = project_root.join("package.json");
        let package_json = read_json(&package_json_path)?;

        let mut names = BTreeSet::new();
        add_dependency_names(&mut names, &package_json, "dependencies");
        add_dependency_names(&mut names, &package_json, "devDependencies");

        let mut repositories = Vec::new();
        for name in names {
            let package_path = dependency_package_path(project_root, &name);
            let dependency_json = match read_json(&package_path) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(repo) = repository_from_package(&dependency_json) {
                if let Some(repository) = parse_github_repository(&repo) {
                    repositories.push(repository);
                }
            }
        }

        Ok(repositories)
    }
}

fn add_dependency_names(target: &mut BTreeSet<String>, package_json: &Value, key: &str) {
    if let Some(deps) = package_json.get(key).and_then(|value| value.as_object()) {
        for name in deps.keys() {
            target.insert(name.to_string());
        }
    }
}

fn dependency_package_path(root: &Path, name: &str) -> PathBuf {
    let mut path = root.join("node_modules");
    for segment in name.split('/') {
        path.push(segment);
    }
    path.join("package.json")
}

fn repository_from_package(package: &Value) -> Option<String> {
    if let Some(repo) = package.get("repository") {
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
    if let Some(Value::String(homepage)) = package.get("homepage") {
        return Some(homepage.clone());
    }
    None
}

fn read_json(path: &Path) -> Result<Value, NodeDiscoveryError> {
    let content = fs::read_to_string(path).map_err(|err| NodeDiscoveryError::Io {
        path: path.display().to_string(),
        source: err,
    })?;
    serde_json::from_str(&content).map_err(|err| NodeDiscoveryError::Json {
        path: path.display().to_string(),
        source: err,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_repositories_from_dependencies() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            json!({
                "dependencies": { "left-pad": "^1.0.0" },
                "devDependencies": { "@scope/pkg": "^1.2.3" }
            })
            .to_string(),
        )
        .unwrap();

        let left_pad_dir = dir.path().join("node_modules/left-pad");
        fs::create_dir_all(&left_pad_dir).unwrap();
        fs::write(
            left_pad_dir.join("package.json"),
            json!({ "repository": "git+https://github.com/left-pad/left-pad.git" }).to_string(),
        )
        .unwrap();

        let scoped_dir = dir.path().join("node_modules/@scope/pkg");
        fs::create_dir_all(&scoped_dir).unwrap();
        fs::write(
            scoped_dir.join("package.json"),
            json!({
                "repository": { "url": "https://github.com/scope/pkg" }
            })
            .to_string(),
        )
        .unwrap();

        let discoverer = NodeDiscoverer::new();
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].name, "left-pad");
        assert_eq!(repos[1].name, "pkg");
    }

    #[test]
    fn skips_packages_without_metadata() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            json!({ "dependencies": { "missing": "^1.0.0" } }).to_string(),
        )
        .unwrap();

        let discoverer = NodeDiscoverer::new();
        let repos = discoverer.discover(dir.path()).unwrap();

        assert!(repos.is_empty());
    }
}
