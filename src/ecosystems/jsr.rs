use std::collections::BTreeSet;

use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use reqwest::StatusCode;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum JsrError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("unexpected status {status}")]
    UnexpectedStatus { status: StatusCode },
}

pub trait JsrFetcher {
    fn fetch_repository_url(&self, package: &str) -> Result<Option<String>, JsrError>;
}

#[derive(Clone)]
pub struct HttpJsrClient {
    client: Client,
    base_url: String,
}

impl Default for HttpJsrClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpJsrClient {
    const DEFAULT_BASE_URL: &'static str = "https://jsr.io";

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

    fn package_url(&self, package: &str) -> String {
        let path = package.trim().trim_start_matches('/');
        if let Some(stripped) = path.strip_prefix('@') {
            format!("{}/%40{}", self.base_url.trim_end_matches('/'), stripped)
        } else {
            format!("{}/{}", self.base_url.trim_end_matches('/'), path)
        }
    }
}

impl JsrFetcher for HttpJsrClient {
    fn fetch_repository_url(&self, package: &str) -> Result<Option<String>, JsrError> {
        let url = self.package_url(package);
        let response = self
            .client
            .get(url)
            .header(ACCEPT, "text/html,application/xhtml+xml")
            .send()?;

        match response.status() {
            StatusCode::NOT_FOUND => Ok(None),
            status if !status.is_success() => Err(JsrError::UnexpectedStatus { status }),
            _ => {
                let body = response.text()?;
                Ok(extract_github_repository(&body))
            }
        }
    }
}

pub fn parse_jsr_specifier(specifier: &str) -> Option<String> {
    let rest = specifier.strip_prefix("jsr:")?;
    normalize_jsr_name(rest)
}

pub fn normalize_jsr_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(idx) = trimmed.rfind('@') {
        let suffix = &trimmed[idx + 1..];
        if idx != 0 && !suffix.contains('/') {
            return Some(trimmed[..idx].to_string());
        }
    }

    Some(trimmed.to_string())
}

pub fn collect_jsr_packages_from_jsr_manifest(value: &Value) -> BTreeSet<String> {
    let mut packages = BTreeSet::new();
    collect_dependency_sections(value, &mut packages);
    collect_import_specifiers(value, &mut packages);
    collect_jsr_strings(value, &mut packages);
    packages
}

pub fn collect_import_specifiers(value: &Value, packages: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(imports) = map.get("imports").and_then(|v| v.as_object()) {
                for (key, value) in imports {
                    if let Some(pkg) = parse_jsr_specifier(key) {
                        packages.insert(pkg);
                    }
                    if let Some(value_str) = value.as_str() {
                        if let Some(pkg) = parse_jsr_specifier(value_str) {
                            packages.insert(pkg);
                        }
                    }
                }
            }

            for child in map.values() {
                collect_import_specifiers(child, packages);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_import_specifiers(item, packages);
            }
        }
        _ => {}
    }
}

pub fn collect_jsr_strings(value: &Value, packages: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            if let Some(pkg) = parse_jsr_specifier(text) {
                packages.insert(pkg);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_jsr_strings(item, packages);
            }
        }
        Value::Object(map) => {
            for child in map.values() {
                collect_jsr_strings(child, packages);
            }
        }
        _ => {}
    }
}

pub fn collect_dependency_sections(value: &Value, packages: &mut BTreeSet<String>) {
    const SECTIONS: [&str; 4] = [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ];

    match value {
        Value::Object(map) => {
            for section in SECTIONS {
                if let Some(deps) = map.get(section).and_then(|v| v.as_object()) {
                    for (name, spec) in deps {
                        if let Some(pkg) = normalize_jsr_name(name) {
                            packages.insert(pkg);
                        }
                        collect_jsr_strings(spec, packages);
                    }
                }
            }

            for child in map.values() {
                collect_dependency_sections(child, packages);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_dependency_sections(item, packages);
            }
        }
        _ => {}
    }
}

fn extract_github_repository(html: &str) -> Option<String> {
    let anchor = Regex::new(r#"<a[^>]*aria-label\s*=\s*\"GitHub repository\"[^>]*>"#)
        .ok()?
        .find(html)?;
    Regex::new(r#"href\s*=\s*\"([^\"]+)\""#)
        .ok()?
        .captures(anchor.as_str())
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn jsr_html(url: &str) -> String {
        format!(
            r#"<html><body><a aria-label="GitHub repository" href="{url}">Repository</a></body></html>"#
        )
    }

    #[test]
    fn fetches_repository_url() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/%40scope/pkg");
            then.status(200)
                .body(jsr_html("https://github.com/scope/pkg"));
        });

        let client = HttpJsrClient::with_base_url(server.base_url());
        let repo = client.fetch_repository_url("@scope/pkg").unwrap().unwrap();
        assert_eq!(repo, "https://github.com/scope/pkg");
    }

    #[test]
    fn handles_missing_repository() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/missing");
            then.status(404);
        });

        let client = HttpJsrClient::with_base_url(server.base_url());
        let repo = client.fetch_repository_url("missing").unwrap();
        assert!(repo.is_none());
    }

    #[test]
    fn parse_jsr_specifier_handles_versions() {
        assert_eq!(
            parse_jsr_specifier("jsr:@scope/name"),
            Some("@scope/name".to_string())
        );
        assert_eq!(
            parse_jsr_specifier("jsr:@scope/name@1.0.0"),
            Some("@scope/name".to_string())
        );
        assert_eq!(
            parse_jsr_specifier("jsr:unscoped@^1"),
            Some("unscoped".to_string())
        );
        assert_eq!(
            parse_jsr_specifier("jsr:unscoped"),
            Some("unscoped".to_string())
        );
    }
}
