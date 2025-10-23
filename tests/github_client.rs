use httpmock::prelude::*;
use serde_json::json;
use thanks_stars::github::{GitHubApi, GitHubClient, GitHubError};

#[test]
fn stars_repository_successfully() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(PUT)
            .path("/user/starred/owner/repo")
            .header("authorization", "token test-token")
            .header("accept", "application/vnd.github.v3+json");
        then.status(204);
    });

    let client = GitHubClient::with_base_url("test-token", server.base_url()).unwrap();
    client.star("owner", "repo").unwrap();
    mock.assert();
}

#[test]
fn surfaces_api_errors() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(PUT).path("/user/starred/owner/repo");
        then.status(401).body("unauthorized");
    });

    let client = GitHubClient::with_base_url("test-token", server.base_url()).unwrap();
    let err = client.star("owner", "repo").unwrap_err();

    match err {
        GitHubError::Api { status, .. } => assert_eq!(status, 401),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn viewer_has_starred_returns_flag() {
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/graphql")
            .header("authorization", "token test-token");
        then.status(200).json_body(json!({
            "data": {"repository": {"viewerHasStarred": true}}
        }));
    });

    let client = GitHubClient::with_base_url("test-token", server.base_url()).unwrap();
    let result = client.viewer_has_starred("owner", "repo").unwrap();
    assert!(result);
    mock.assert();
}

#[test]
fn viewer_has_starred_surfaces_errors() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(POST).path("/graphql");
        then.status(200).json_body(json!({
            "errors": [{"message": "boom"}]
        }));
    });

    let client = GitHubClient::with_base_url("test-token", server.base_url()).unwrap();
    let err = client.viewer_has_starred("owner", "repo").unwrap_err();

    match err {
        GitHubError::Api { body, .. } => assert!(body.contains("boom")),
        other => panic!("unexpected error: {other:?}"),
    }
}
