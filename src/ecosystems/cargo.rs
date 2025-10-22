use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

use crate::discovery::{parse_github_repository, Repository};

#[derive(Debug, thiserror::Error)]
pub enum CargoDiscoveryError {
    #[error("failed to run `cargo metadata`: {0}")]
    CommandFailed(String),
    #[error("failed to execute `cargo metadata`: {0}")]
    CommandIo(#[from] std::io::Error),
    #[error("failed to parse cargo metadata: {0}")]
    Json(#[from] serde_json::Error),
}

pub trait MetadataFetcher {
    fn fetch(&self, project_root: &Path) -> Result<String, CargoDiscoveryError>;
}

#[derive(Default)]
pub struct CommandMetadataFetcher;

impl MetadataFetcher for CommandMetadataFetcher {
    fn fetch(&self, project_root: &Path) -> Result<String, CargoDiscoveryError> {
        let output = Command::new("cargo")
            .current_dir(project_root)
            .args(["metadata", "--format-version", "1"])
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(CargoDiscoveryError::CommandFailed(stderr));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

pub struct CargoDiscoverer<F: MetadataFetcher> {
    fetcher: F,
}

impl<F: MetadataFetcher> CargoDiscoverer<F> {
    pub fn new(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, CargoDiscoveryError> {
        let metadata_json = self.fetcher.fetch(project_root)?;
        let metadata: Metadata = serde_json::from_str(&metadata_json)?;
        let Metadata {
            packages,
            resolve,
            workspace_members,
        } = metadata;

        let mut dependency_ids = BTreeSet::new();
        if let Some(resolve) = resolve {
            let node_map: HashMap<_, _> = resolve
                .nodes
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect();

            for member in workspace_members {
                if let Some(node) = node_map.get(&member) {
                    for dep in &node.deps {
                        dependency_ids.insert(dep.pkg.clone());
                    }
                }
            }
        }

        let package_map: HashMap<_, _> = packages
            .into_iter()
            .map(|package| (package.id.clone(), package))
            .collect();

        let mut repositories = Vec::new();
        for id in dependency_ids {
            if let Some(package) = package_map.get(&id) {
                if let Some(repo) = &package.repository {
                    if let Some(mut repository) = parse_github_repository(repo) {
                        repository.via = Some("Cargo.toml".to_string());
                        repositories.push(repository);
                    }
                }
            }
        }

        Ok(repositories)
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    #[serde(default)]
    resolve: Option<Resolve>,
    #[serde(default)]
    workspace_members: Vec<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    repository: Option<String>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    pkg: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    struct StaticMetadataFetcher {
        json: String,
    }

    impl MetadataFetcher for StaticMetadataFetcher {
        fn fetch(&self, _project_root: &Path) -> Result<String, CargoDiscoveryError> {
            Ok(self.json.clone())
        }
    }

    #[test]
    fn extracts_repositories_from_metadata() {
        let metadata = r#"{
            "packages": [
                {
                    "id": "root 0.1.0 (path+file:///root)",
                    "repository": null
                },
                {
                    "id": "dep1 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)",
                    "repository": "https://github.com/example/dep1"
                },
                {
                    "id": "dep2 2.0.0 (git+https://github.com/example/dep2)",
                    "repository": "https://github.com/example/dep2"
                }
            ],
            "workspace_members": ["root 0.1.0 (path+file:///root)"],
            "resolve": {
                "nodes": [
                    {
                        "id": "root 0.1.0 (path+file:///root)",
                        "deps": [
                            { "pkg": "dep1 1.0.0 (registry+https://github.com/rust-lang/crates.io-index)" },
                            { "pkg": "dep2 2.0.0 (git+https://github.com/example/dep2)" }
                        ]
                    }
                ]
            }
        }"#;

        let discoverer = CargoDiscoverer::new(StaticMetadataFetcher {
            json: metadata.to_string(),
        });

        let repos = discoverer.discover(Path::new(".")).unwrap();
        assert_eq!(repos.len(), 2);
        let names: Vec<_> = repos.iter().map(|repo| repo.name.as_str()).collect();
        assert!(names.contains(&"dep1"));
        assert!(names.contains(&"dep2"));
    }

    #[test]
    fn returns_empty_when_no_repositories() {
        let metadata = r#"{
            "packages": [
                { "id": "root 0.1.0 (path+file:///root)", "repository": null }
            ],
            "workspace_members": ["root 0.1.0 (path+file:///root)"],
            "resolve": { "nodes": [ { "id": "root 0.1.0 (path+file:///root)", "deps": [] } ] }
        }"#;

        let discoverer = CargoDiscoverer::new(StaticMetadataFetcher {
            json: metadata.to_string(),
        });

        let repos = discoverer.discover(Path::new(".")).unwrap();
        assert!(repos.is_empty());
    }
}
