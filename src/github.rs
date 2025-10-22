use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("failed to build HTTP client: {0}")]
    ClientBuild(#[from] reqwest::Error),
    #[error("GitHub API responded with status {status}: {body}")]
    Api { status: u16, body: String },
}

pub trait GitHubApi {
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
