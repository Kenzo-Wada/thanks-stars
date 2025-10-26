use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use jsonc_parser::{errors::ParseError, parse_to_serde_value, ParseOptions};
use serde_json::Value;

use crate::discovery::{parse_github_repository, Repository};
use crate::ecosystems::jsr::{
    collect_import_specifiers, collect_jsr_packages_from_jsr_manifest, collect_jsr_strings,
    normalize_jsr_name, parse_jsr_specifier, HttpJsrClient, JsrError, JsrFetcher,
};

#[derive(Debug, thiserror::Error)]
pub enum DenoDiscoveryError {
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
    #[error("failed to parse {path} as JSONC: {source}")]
    Jsonc {
        path: String,
        #[source]
        source: ParseError,
    },
    #[error("failed to fetch repository for {package}: {source}")]
    Jsr {
        package: String,
        #[source]
        source: JsrError,
    },
}

pub struct DenoDiscoverer<F: JsrFetcher> {
    fetcher: F,
}

impl Default for DenoDiscoverer<HttpJsrClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl DenoDiscoverer<HttpJsrClient> {
    pub fn new() -> Self {
        Self {
            fetcher: HttpJsrClient::new(),
        }
    }
}

impl<F: JsrFetcher> DenoDiscoverer<F> {
    pub fn with_fetcher(fetcher: F) -> Self {
        Self { fetcher }
    }

    pub fn discover(&self, project_root: &Path) -> Result<Vec<Repository>, DenoDiscoveryError> {
        let mut packages = BTreeMap::new();

        collect_packages_from_deno_lock(project_root, &mut packages)?;
        collect_packages_from_deno_config(project_root, "deno.json", &mut packages)?;
        collect_packages_from_deno_config(project_root, "deno.jsonc", &mut packages)?;
        collect_packages_from_jsr_manifest(project_root, &mut packages)?;

        let mut repositories = Vec::new();
        for (package, via) in packages {
            let package_for_error = package.clone();
            let Some(url) = self
                .fetcher
                .fetch_repository_url(&package)
                .map_err(|source| DenoDiscoveryError::Jsr {
                    package: package_for_error,
                    source,
                })?
            else {
                continue;
            };

            if let Some(mut repository) = parse_github_repository(&url) {
                repository.via = Some(via);
                repositories.push(repository);
            }
        }

        Ok(repositories)
    }
}

fn collect_packages_from_deno_lock(
    project_root: &Path,
    packages: &mut BTreeMap<String, String>,
) -> Result<(), DenoDiscoveryError> {
    let lock_path = project_root.join("deno.lock");
    if !lock_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(&lock_path).map_err(|source| DenoDiscoveryError::Io {
        path: lock_path.display().to_string(),
        source,
    })?;

    let json: Value =
        serde_json::from_str(&content).map_err(|source| DenoDiscoveryError::Json {
            path: lock_path.display().to_string(),
            source,
        })?;

    for package in collect_jsr_packages_from_lock(&json) {
        insert_package(packages, package, "deno.lock");
    }

    Ok(())
}

fn collect_packages_from_deno_config(
    project_root: &Path,
    file_name: &str,
    packages: &mut BTreeMap<String, String>,
) -> Result<(), DenoDiscoveryError> {
    let config_path = project_root.join(file_name);
    if !config_path.exists() {
        return Ok(());
    }

    let value = parse_jsonc_file(&config_path)?;
    for package in collect_jsr_packages_from_deno_config(&value) {
        insert_package(packages, package, file_name);
    }

    Ok(())
}

fn collect_packages_from_jsr_manifest(
    project_root: &Path,
    packages: &mut BTreeMap<String, String>,
) -> Result<(), DenoDiscoveryError> {
    let manifest_path = project_root.join("jsr.json");
    if !manifest_path.exists() {
        return Ok(());
    }

    let value = parse_jsonc_file(&manifest_path)?;
    for package in collect_jsr_packages_from_jsr_manifest(&value) {
        insert_package(packages, package, "jsr.json");
    }

    Ok(())
}

fn insert_package(packages: &mut BTreeMap<String, String>, package: String, via: &str) {
    packages.entry(package).or_insert_with(|| via.to_string());
}

fn collect_jsr_packages_from_lock(value: &Value) -> BTreeSet<String> {
    let mut packages = BTreeSet::new();

    if let Some(specifiers) = value
        .get("packages")
        .and_then(|p| p.get("specifiers"))
        .and_then(|s| s.as_object())
    {
        for (key, value) in specifiers {
            if let Some(pkg) = parse_jsr_specifier(key) {
                packages.insert(pkg);
            }
            if let Some(resolved) = value.as_str() {
                if let Some(pkg) = parse_jsr_specifier(resolved) {
                    packages.insert(pkg);
                }
            }
        }
    }

    if let Some(jsr_packages) = value
        .get("packages")
        .and_then(|p| p.get("jsr"))
        .and_then(|j| j.as_object())
    {
        for (key, pkg_value) in jsr_packages {
            if let Some(pkg) = normalize_jsr_name(key) {
                packages.insert(pkg);
            }
            if let Some(dependencies) = pkg_value
                .get("dependencies")
                .and_then(|deps| deps.as_object())
            {
                for dep in dependencies.values() {
                    if let Some(dep_str) = dep.as_str() {
                        if let Some(pkg) = parse_jsr_specifier(dep_str) {
                            packages.insert(pkg);
                        }
                    }
                }
            }
        }
    }

    packages
}

fn collect_jsr_packages_from_deno_config(value: &Value) -> BTreeSet<String> {
    let mut packages = BTreeSet::new();
    collect_import_specifiers(value, &mut packages);
    collect_jsr_strings(value, &mut packages);
    packages
}

fn parse_jsonc_file(path: &Path) -> Result<Value, DenoDiscoveryError> {
    let content = fs::read_to_string(path).map_err(|source| DenoDiscoveryError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let parse_options = ParseOptions::default();
    let value = parse_to_serde_value(&content, &parse_options).map_err(|source| {
        DenoDiscoveryError::Jsonc {
            path: path.display().to_string(),
            source,
        }
    })?;
    Ok(value.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use std::fs;
    use tempfile::tempdir;

    use crate::ecosystems::jsr::HttpJsrClient;

    fn jsr_html(url: &str) -> String {
        format!(
            r#"<html><body><a aria-label="GitHub repository" href="{url}">Repository</a></body></html>"#
        )
    }

    #[test]
    fn discovers_jsr_packages_from_deno_lock() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.lock"),
            r#"{
  "version": "3",
  "packages": {
    "specifiers": {
      "jsr:@scope/pkg": "jsr:@scope/pkg@1.2.3",
      "jsr:unscoped@^2": "jsr:unscoped@2.0.0",
      "npm:chalk": "npm:chalk@5.0.0"
    },
    "jsr": {
      "@scope/pkg@1.2.3": {
        "dependencies": {
          "dep": "jsr:@other/dep@0.1.0"
        }
      }
    }
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/pkg");
            then.status(200)
                .body(jsr_html("https://github.com/scope/pkg"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/unscoped");
            then.status(200)
                .body(jsr_html("https://github.com/example/unscoped"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/%40other/dep");
            then.status(200)
                .body(jsr_html("https://github.com/other/dep"));
        });

        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 3);
        assert_eq!(repos[0].name, "dep");
        assert_eq!(repos[1].name, "pkg");
        assert_eq!(repos[2].name, "unscoped");
        assert!(repos
            .iter()
            .all(|repo| repo.via.as_deref() == Some("deno.lock")));
    }

    #[test]
    fn skips_packages_without_repository_links() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.lock"),
            r#"{
  "version": "3",
  "packages": {
    "specifiers": {
      "jsr:@scope/pkg": "jsr:@scope/pkg@1.0.0"
    }
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/pkg");
            then.status(200).body("<html><body>No repo</body></html>");
        });

        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn discovers_packages_from_deno_json() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.json"),
            r#"{
  "imports": {
    "jsr:@scope/pkg": "jsr:@scope/pkg@1.0.0",
    "@std/assert": "jsr:@std/assert@^1"
  },
  "compilerOptions": {
    "types": ["jsr:@types/testing@0.1"]
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/pkg");
            then.status(200)
                .body(jsr_html("https://github.com/scope/pkg"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/%40std/assert");
            then.status(200)
                .body(jsr_html("https://github.com/std/assert"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/%40types/testing");
            then.status(200)
                .body(jsr_html("https://github.com/types/testing"));
        });

        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 3);
        assert_eq!(repos[0].name, "assert");
        assert_eq!(repos[1].name, "pkg");
        assert_eq!(repos[2].name, "testing");
        assert!(repos
            .iter()
            .all(|repo| repo.via.as_deref() == Some("deno.json")));
    }

    #[test]
    fn discovers_packages_from_deno_jsonc() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.jsonc"),
            r#"{
  // comment
  "imports": {
    "jsr:@jsonc/pkg": "jsr:@jsonc/pkg@0.1.0", // trailing
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40jsonc/pkg");
            then.status(200)
                .body(jsr_html("https://github.com/jsonc/pkg"));
        });

        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].name, "pkg");
        assert_eq!(repos[0].via.as_deref(), Some("deno.jsonc"));
    }

    #[test]
    fn discovers_packages_from_jsr_manifest() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("jsr.json"),
            r#"{
  "name": "@scope/example",
  "version": "1.0.0",
  "dependencies": {
    "@std/assert": "^1.0.0",
    "unscoped": "jsr:unscoped@^2"
  },
  "devDependencies": {
    "@scope/dev": "^0.1.0"
  },
  "imports": {
    "helper": "jsr:@scope/helper@1"
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40std/assert");
            then.status(200)
                .body(jsr_html("https://github.com/std/assert"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/unscoped");
            then.status(200)
                .body(jsr_html("https://github.com/example/unscoped"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/dev");
            then.status(200)
                .body(jsr_html("https://github.com/scope/dev"));
        });
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/helper");
            then.status(200)
                .body(jsr_html("https://github.com/scope/helper"));
        });

        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let mut repos = discoverer.discover(dir.path()).unwrap();
        repos.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(repos.len(), 4);
        assert_eq!(repos[0].name, "assert");
        assert_eq!(repos[1].name, "dev");
        assert_eq!(repos[2].name, "helper");
        assert_eq!(repos[3].name, "unscoped");
        assert!(repos
            .iter()
            .all(|repo| repo.via.as_deref() == Some("jsr.json")));
    }

    #[test]
    fn ignores_non_jsr_entries() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("deno.lock"),
            r#"{
  "version": "3",
  "packages": {
    "specifiers": {
      "npm:chalk": "npm:chalk@5.0.0",
      "https://deno.land/x/example@1.0.0/mod.ts": "https://deno.land/x/example@1.0.0/mod.ts"
    }
  }
}"#,
        )
        .unwrap();

        let server = MockServer::start();
        let discoverer =
            DenoDiscoverer::with_fetcher(HttpJsrClient::with_base_url(server.base_url()));
        let repos = discoverer.discover(dir.path()).unwrap();
        assert!(repos.is_empty());
    }
}
