use httpmock::prelude::*;
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
