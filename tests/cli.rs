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
    let graphql = server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .header("authorization", "token cli-token");
        then.status(200).json_body(json!({
            "data": {"repository": {"viewerHasStarred": false}}
        }));
    });

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
            "⭐ Starred https://github.com/example/dep via package.json",
        ))
        .stdout(predicate::str::contains(
            "✨ Completed! Starred 1 repository.",
        ));

    mock.assert();
    graphql.assert();
}

#[test]
fn run_command_dry_run_skips_starring() {
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
    let graphql = server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .header("authorization", "token cli-token");
        then.status(200).json_body(json!({
            "data": {"repository": {"viewerHasStarred": false}}
        }));
    });

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
        .arg("run")
        .arg("--dry-run");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "⭐ Would star https://github.com/example/dep via package.json",
        ))
        .stdout(predicate::str::contains(
            "✨ Dry run complete! 1 repository would be starred.",
        ));

    assert_eq!(mock.calls(), 0);
    graphql.assert();
}

#[test]
fn default_command_accepts_dry_run_flag() {
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
    let graphql = server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .header("authorization", "token cli-token");
        then.status(200).json_body(json!({
            "data": {"repository": {"viewerHasStarred": false}}
        }));
    });

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
        .arg("--dry-run");

    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "⭐ Would star https://github.com/example/dep via package.json",
        ))
        .stdout(predicate::str::contains(
            "✨ Dry run complete! 1 repository would be starred.",
        ));

    assert_eq!(mock.calls(), 0);
    graphql.assert();
}

#[test]
fn run_command_reports_already_starred() {
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
    let graphql = server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .header("authorization", "token cli-token");
        then.status(200).json_body(json!({
            "data": {"repository": {"viewerHasStarred": true}}
        }));
    });

    let star_mock = server.mock(|when, _then| {
        when.method(PUT)
            .path("/user/starred/example/dep")
            .header("authorization", "token cli-token");
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
            "⭐ Already starred https://github.com/example/dep (already starred) via package.json",
        ))
        .stdout(predicate::str::contains(
            "✨ Completed! All 1 repository were already starred.",
        ));

    star_mock.assert_calls(0);
    graphql.assert();
}
