//! The few GitHub calls a cloud task needs: open a draft PR, keep its
//! description current, comment, and mark it ready for review.

use serde::Deserialize;
use serde_json::json;

use crate::CloudError;

pub struct GitHub {
    http: reqwest::Client,
    api: String,
    token: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub html_url: String,
    /// GraphQL id, needed to mark the PR ready for review.
    pub node_id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged_at: Option<String>,
}

impl GitHub {
    pub fn new(api_url: &str, token: &str) -> Result<Self, CloudError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("mira-cloud/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| CloudError::GitHub(e.to_string()))?;
        Ok(Self {
            http,
            api: api_url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.api))
            .bearer_auth(&self.token)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    async fn send(
        &self,
        what: &str,
        rb: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, CloudError> {
        let resp = rb
            .send()
            .await
            .map_err(|e| CloudError::GitHub(format!("{what}: {e}")))?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(CloudError::GitHub(format!("{what}: HTTP {status}: {body}")))
    }

    pub async fn create_draft_pr(
        &self,
        owner: &str,
        repo: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<PullRequest, CloudError> {
        let resp = self
            .send(
                "creating the pull request",
                self.req(reqwest::Method::POST, &format!("/repos/{owner}/{repo}/pulls"))
                    .json(&json!({ "title": title, "head": head, "base": base, "body": body, "draft": true })),
            )
            .await?;
        resp.json()
            .await
            .map_err(|e| CloudError::GitHub(e.to_string()))
    }

    pub async fn update_body(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), CloudError> {
        self.send(
            "updating the pull request",
            self.req(
                reqwest::Method::PATCH,
                &format!("/repos/{owner}/{repo}/pulls/{number}"),
            )
            .json(&json!({ "body": body })),
        )
        .await
        .map(drop)
    }

    pub async fn comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), CloudError> {
        self.send(
            "commenting",
            self.req(
                reqwest::Method::POST,
                &format!("/repos/{owner}/{repo}/issues/{number}/comments"),
            )
            .json(&json!({ "body": body })),
        )
        .await
        .map(drop)
    }

    /// GET a REST path as JSON.
    pub async fn get(&self, path: &str) -> Result<serde_json::Value, CloudError> {
        self.send(path, self.req(reqwest::Method::GET, path))
            .await?
            .json()
            .await
            .map_err(|e| CloudError::GitHub(e.to_string()))
    }

    /// POST JSON to a REST path.
    pub async fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, CloudError> {
        let resp = self
            .send(path, self.req(reqwest::Method::POST, path).json(body))
            .await?;
        Ok(resp.json().await.unwrap_or(serde_json::Value::Null))
    }

    /// A pull request's unified diff.
    pub async fn pr_diff(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<String, CloudError> {
        self.send(
            "fetching the diff",
            // Its own Accept: `req` already sets the JSON one, and a second
            // header would be appended rather than replace it.
            self.http
                .get(format!("{}/repos/{owner}/{repo}/pulls/{number}", self.api))
                .bearer_auth(&self.token)
                .header("Accept", "application/vnd.github.v3.diff")
                .header("X-GitHub-Api-Version", "2022-11-28"),
        )
        .await?
        .text()
        .await
        .map_err(|e| CloudError::GitHub(e.to_string()))
    }

    /// Draft → ready. REST can't do this; it's a GraphQL mutation.
    pub async fn mark_ready(&self, node_id: &str) -> Result<(), CloudError> {
        let resp = self
            .send(
                "marking the pull request ready",
                self.req(reqwest::Method::POST, "/graphql").json(&json!({
                    "query": "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { pullRequest { isDraft } } }",
                    "variables": { "id": node_id },
                })),
            )
            .await?;
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| CloudError::GitHub(e.to_string()))?;
        if let Some(errs) = v.get("errors") {
            return Err(CloudError::GitHub(format!("marking ready: {errs}")));
        }
        Ok(())
    }

    /// The PR (open or closed) whose head is `branch`, if any.
    pub async fn find_pr(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Option<PullRequest>, CloudError> {
        let resp = self
            .send(
                "looking up the pull request",
                self.req(
                    reqwest::Method::GET,
                    &format!("/repos/{owner}/{repo}/pulls"),
                )
                .query(&[
                    ("head", format!("{owner}:{branch}")),
                    ("state", "all".into()),
                ]),
            )
            .await?;
        let prs: Vec<PullRequest> = resp
            .json()
            .await
            .map_err(|e| CloudError::GitHub(e.to_string()))?;
        Ok(prs.into_iter().next())
    }
}
