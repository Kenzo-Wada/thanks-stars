use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("failed to build HTTP client: {0}")]
    ClientBuild(#[from] reqwest::Error),
    #[error("GitHub API responded with status {status}: {body}")]
    Api { status: u16, body: String },
}

pub trait GitHubApi {
    fn viewer_has_starred(&self, owner: &str, repo: &str) -> Result<bool, GitHubError>;
    fn star(&self, owner: &str, repo: &str) -> Result<(), GitHubError>;
}

pub struct GitHubClient {
    token: String,
    client: Client,
    base_url: String,
}

impl GitHubClient {
    pub fn new(token: impl Into<String>) -> Result<Self, GitHubError> {
        Self::with_base_url(token, "https://api.github.com")
    }

    pub fn with_base_url(
        token: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, GitHubError> {
        let token = token.into();
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let client = Client::builder().user_agent("thanks-stars").build()?;
        Ok(Self {
            token,
            client,
            base_url,
        })
    }

    fn auth_header(&self) -> String {
        format!("token {}", self.token)
    }
}

impl GitHubApi for GitHubClient {
    fn viewer_has_starred(&self, owner: &str, repo: &str) -> Result<bool, GitHubError> {
        let url = format!("{}/graphql", self.base_url);
        let query = serde_json::json!({
            "query": "query($owner:String!,$name:String!){repository(owner:$owner,name:$name){viewerHasStarred}}",
            "variables": {"owner": owner, "name": repo}
        });

        let response = self
            .client
            .post(url)
            .header(USER_AGENT, "thanks-stars")
            .header(ACCEPT, "application/vnd.github+json")
            .header(AUTHORIZATION, self.auth_header())
            .json(&query)
            .send()
            .map_err(GitHubError::from)?;

        let status = response.status();
        let body = response.bytes().map_err(GitHubError::from)?;

        if !status.is_success() {
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }

        let parsed: GraphqlResponse =
            serde_json::from_slice(&body).map_err(|err| GitHubError::Api {
                status: status.as_u16(),
                body: format!(
                    "failed to parse GraphQL response: {err}; body: {}",
                    String::from_utf8_lossy(&body)
                ),
            })?;

        if let Some(errors) = parsed.errors {
            let message = errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(GitHubError::Api {
                status: status.as_u16(),
                body: message,
            });
        }

        let repo_data = parsed
            .data
            .and_then(|data| data.repository)
            .ok_or_else(|| GitHubError::Api {
                status: status.as_u16(),
                body: "repository data missing from GraphQL response".to_string(),
            })?;

        Ok(repo_data.viewer_has_starred)
    }

    fn star(&self, owner: &str, repo: &str) -> Result<(), GitHubError> {
        let url = format!("{}/user/starred/{}/{}", self.base_url, owner, repo);
        let response = self
            .client
            .put(url)
            .header(USER_AGENT, "thanks-stars")
            .header(ACCEPT, "application/vnd.github.v3+json")
            .header(AUTHORIZATION, self.auth_header())
            .send()
            .map_err(GitHubError::from)?;

        if response.status().is_success() || response.status().as_u16() == 304 {
            return Ok(());
        }

        let status = response.status().as_u16();
        let body = response.text().unwrap_or_default();
        Err(GitHubError::Api { status, body })
    }
}

#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: Option<GraphqlData>,
    errors: Option<Vec<GraphqlErrorMessage>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlData {
    repository: Option<GraphqlRepository>,
}

#[derive(Debug, Deserialize)]
struct GraphqlRepository {
    #[serde(rename = "viewerHasStarred")]
    viewer_has_starred: bool,
}

#[derive(Debug, Deserialize)]
struct GraphqlErrorMessage {
    message: String,
}
