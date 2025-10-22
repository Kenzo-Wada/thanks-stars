use assert_cmd::Command;
use httpmock::prelude::*;
use predicates::prelude::*;
use serde_json::json;
use std::fs;
use tempfile::tempdir;

#[test]
fn auth_command_saves_token() {
    let dir = tempdir().unwrap();
    let mut cmd = Command::cargo_bin("thanks-stars").unwrap();
    cmd.env("THANKS_STARS_CONFIG_DIR", dir.path())
        .arg("auth")
        .arg("--token")
        .arg("abc123");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Token saved"));

    let config_path = dir.path().join("config.toml");
    assert!(config_path.exists());
    let contents = fs::read_to_string(config_path).unwrap();
    assert!(contents.contains("abc123"));
}

#[test]
fn run_command_stars_dependencies() {
    let project = tempdir().unwrap();
    fs::write(
        project.path().join("package.json"),
        json!({ "dependencies": { "dep": "^1.0.0" } }).to_string(),
    )
    .unwrap();
    let dep_dir = project.path().join("node_modules/dep");
    fs::create_dir_all(&dep_dir).unwrap();
    fs::write(
        dep_dir.join("package.json"),
        json!({ "repository": "https://github.com/example/dep" }).to_string(),
    )
    .unwrap();

    let server = httpmock::MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PUT)
            .path("/user/starred/example/dep")
            .header("authorization", "token cli-token");
        then.status(204);
    });

    let mut cmd = Command::cargo_bin("thanks-stars").unwrap();
    cmd.env("THANKS_STARS_API_BASE", server.base_url())
        .env("GITHUB_TOKEN", "cli-token")
        .env("NO_COLOR", "1")
        .current_dir(project.path())
        .arg("run");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "⭐ Starred https://github.com/example/dep",
        ))
        .stdout(predicate::str::contains(
            "✨ Completed! Starred 1 repositories.",
        ));

    mock.assert();
}
