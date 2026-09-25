//! Connect a repository to Mira's GitHub Action with no files to edit.
//!
//! [`connect`] does over the GitHub API what someone would otherwise do
//! by hand:
//!
//! 1. stores the model API key as the `MIRA_API_KEY` Actions secret
//!    (encrypted with the repository's public key, as GitHub requires);
//! 2. stores the provider, model and endpoint as Actions variables;
//! 3. adds `.github/workflows/mira.yml`, committed to the default branch,
//!    or, when the branch is protected, on a branch with a pull request
//!    to merge.
//!
//! The token needs the `repo` and `workflow` scopes (classic), or
//! Contents, Workflows, Secrets, Variables and Pull requests write
//! (fine-grained).

use base64::Engine as _;
use serde::Serialize;
use serde_json::{json, Value};

use crate::github::GitHub;
use crate::CloudError;

pub const WORKFLOW_PATH: &str = ".github/workflows/mira.yml";
/// The action the workflow uses.
pub const ACTION_REF: &str = "runmira/mira@main";

/// What to connect the repository with.
pub struct ConnectOptions {
    pub provider: String,
    pub model: String,
    /// Only for providers Mira doesn't know by name.
    pub base_url: Option<String>,
    pub api_key: String,
    /// Where the action comes from, e.g. `runmira/mira@main`.
    pub action_ref: String,
}

impl ConnectOptions {
    /// Options from the provider settings Mira itself uses. The endpoint
    /// is only stored when it isn't the provider's usual one.
    pub fn new(
        provider: &str,
        model: &str,
        base_url: Option<&str>,
        api_key: &str,
    ) -> Result<Self, String> {
        if api_key.trim().is_empty() {
            return Err(format!(
                "no API key for {provider}: GitHub Actions needs one (set it in Settings or with `mira init`)"
            ));
        }
        if model.trim().is_empty() {
            return Err("no model chosen: pick one in Settings or pass --model".into());
        }
        let default = mira_config::default_base_url_for(provider);
        let base_url = base_url
            .filter(|u| {
                !u.is_empty()
                    && Some(u.trim_end_matches('/')) != default.map(|d| d.trim_end_matches('/'))
            })
            .map(str::to_owned);
        Ok(Self {
            provider: provider.to_owned(),
            model: model.to_owned(),
            base_url,
            api_key: api_key.to_owned(),
            action_ref: ACTION_REF.to_owned(),
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectReport {
    pub repo: String,
    /// The workflow was committed straight to this branch…
    pub committed_to: Option<String>,
    /// …or proposed in this pull request, because the branch is protected.
    pub pull_request: Option<String>,
    /// Already there and identical: nothing to commit.
    pub workflow_unchanged: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub repo: String,
    pub workflow: bool,
    pub api_key_secret: bool,
}

/// Whether `owner/repo` is set up.
pub async fn status(gh: &GitHub, owner: &str, repo: &str) -> Result<Status, CloudError> {
    let workflow = gh
        .get(&format!("/repos/{owner}/{repo}/contents/{WORKFLOW_PATH}"))
        .await
        .is_ok();
    let api_key_secret = gh
        .get(&format!(
            "/repos/{owner}/{repo}/actions/secrets/MIRA_API_KEY"
        ))
        .await
        .is_ok();
    Ok(Status {
        repo: format!("{owner}/{repo}"),
        workflow,
        api_key_secret,
    })
}

pub async fn connect(
    gh: &GitHub,
    owner: &str,
    repo: &str,
    opts: &ConnectOptions,
) -> Result<ConnectReport, CloudError> {
    let info = gh.get(&format!("/repos/{owner}/{repo}")).await?;
    if info["permissions"]["admin"] == false && info["permissions"]["maintain"] == false {
        // Secrets and variables need admin (or maintain) on the repo.
        return Err(CloudError::GitHub(format!(
            "your token can't manage {owner}/{repo}'s secrets; you need admin access to it"
        )));
    }
    let branch = info["default_branch"].as_str().unwrap_or("main").to_owned();

    set_secret(gh, owner, repo, "MIRA_API_KEY", &opts.api_key).await?;
    set_variable(gh, owner, repo, "MIRA_PROVIDER", &opts.provider).await?;
    set_variable(gh, owner, repo, "MIRA_MODEL", &opts.model).await?;
    match opts.base_url.as_deref().filter(|u| !u.is_empty()) {
        Some(url) => set_variable(gh, owner, repo, "MIRA_BASE_URL", url).await?,
        None => {
            let _ = gh
                .delete(&format!(
                    "/repos/{owner}/{repo}/actions/variables/MIRA_BASE_URL"
                ))
                .await;
        }
    }

    let content = workflow(&opts.action_ref);
    let path = format!("/repos/{owner}/{repo}/contents/{WORKFLOW_PATH}");
    let existing = gh.get(&format!("{path}?ref={branch}")).await.ok();
    if let Some(e) = &existing {
        let current = e["content"]
            .as_str()
            .map(|c| c.replace('\n', ""))
            .and_then(|c| base64::engine::general_purpose::STANDARD.decode(c).ok());
        if current.as_deref() == Some(content.as_bytes()) {
            return Ok(ConnectReport {
                repo: format!("{owner}/{repo}"),
                committed_to: None,
                pull_request: None,
                workflow_unchanged: true,
            });
        }
    }
    let mut put = json!({
        "message": "Add the Mira workflow",
        "content": base64::engine::general_purpose::STANDARD.encode(&content),
        "branch": branch,
    });
    if let Some(sha) = existing.as_ref().and_then(|e| e["sha"].as_str()) {
        put["sha"] = json!(sha);
    }
    match gh.put(&path, &put).await {
        Ok(_) => Ok(ConnectReport {
            repo: format!("{owner}/{repo}"),
            committed_to: Some(branch),
            pull_request: None,
            workflow_unchanged: false,
        }),
        // Protected branch (or a rule requiring pull requests): propose it.
        Err(CloudError::GitHub(msg))
            if ["HTTP 409", "HTTP 422", "HTTP 403"]
                .iter()
                .any(|c| msg.contains(c)) =>
        {
            let pr = propose(gh, owner, repo, &branch, &path, put).await?;
            Ok(ConnectReport {
                repo: format!("{owner}/{repo}"),
                committed_to: None,
                pull_request: Some(pr),
                workflow_unchanged: false,
            })
        }
        Err(e) => Err(e),
    }
}

/// Commit the workflow on a new branch and open a pull request.
async fn propose(
    gh: &GitHub,
    owner: &str,
    repo: &str,
    base: &str,
    path: &str,
    mut put: Value,
) -> Result<String, CloudError> {
    let head = gh
        .get(&format!("/repos/{owner}/{repo}/git/ref/heads/{base}"))
        .await?;
    let sha = head["object"]["sha"].as_str().unwrap_or_default();
    let branch = "mira/setup";
    let _ = gh
        .post(
            &format!("/repos/{owner}/{repo}/git/refs"),
            &json!({"ref": format!("refs/heads/{branch}"), "sha": sha}),
        )
        .await;
    put["branch"] = json!(branch);
    if let Ok(e) = gh.get(&format!("{path}?ref={branch}")).await {
        if let Some(sha) = e["sha"].as_str() {
            put["sha"] = json!(sha);
        }
    }
    gh.put(path, &put).await?;
    let pr = gh
        .post(
            &format!("/repos/{owner}/{repo}/pulls"),
            &json!({
                "title": "Set up Mira",
                "head": branch,
                "base": base,
                "body": "Adds the workflow that runs Mira: it reviews pull requests, and works \
                         on `@mira` requests in issues and pull requests. The model key and \
                         settings are already stored as Actions secrets and variables.",
            }),
        )
        .await?;
    Ok(pr["html_url"].as_str().unwrap_or_default().to_owned())
}

/// Store an Actions secret, sealed with the repository's public key.
pub async fn set_secret(
    gh: &GitHub,
    owner: &str,
    repo: &str,
    name: &str,
    value: &str,
) -> Result<(), CloudError> {
    let key = gh
        .get(&format!("/repos/{owner}/{repo}/actions/secrets/public-key"))
        .await?;
    let (Some(key_id), Some(public)) = (key["key_id"].as_str(), key["key"].as_str()) else {
        return Err(CloudError::GitHub(
            "no public key for Actions secrets".into(),
        ));
    };
    let sealed = seal(public, value)?;
    gh.put(
        &format!("/repos/{owner}/{repo}/actions/secrets/{name}"),
        &json!({"encrypted_value": sealed, "key_id": key_id}),
    )
    .await
    .map(drop)
}

/// libsodium `crypto_box_seal`, base64 in and out, as GitHub's secrets
/// API expects.
fn seal(public_b64: &str, value: &str) -> Result<String, CloudError> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let bytes: [u8; 32] = b64
        .decode(public_b64)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| CloudError::GitHub("the repository's public key is malformed".into()))?;
    let public = crypto_box::PublicKey::from(bytes);
    let sealed = public
        .seal(&mut crypto_box::aead::OsRng, value.as_bytes())
        .map_err(|_| CloudError::GitHub("couldn't encrypt the secret".into()))?;
    Ok(b64.encode(sealed))
}

/// Create or update an Actions variable.
pub async fn set_variable(
    gh: &GitHub,
    owner: &str,
    repo: &str,
    name: &str,
    value: &str,
) -> Result<(), CloudError> {
    let body = json!({"name": name, "value": value});
    let path = format!("/repos/{owner}/{repo}/actions/variables/{name}");
    if gh.get(&path).await.is_ok() {
        gh.patch(&path, &body).await.map(drop)
    } else {
        gh.post(&format!("/repos/{owner}/{repo}/actions/variables"), &body)
            .await
            .map(drop)
    }
}

/// The workflow file. Settings come from the secret and variables, so it
/// never needs editing.
pub fn workflow(action_ref: &str) -> String {
    format!(
        r#"# Added by Mira. Settings live in this repository's Actions secrets and
# variables (MIRA_API_KEY, MIRA_PROVIDER, MIRA_MODEL); change them with
# `mira github setup` or in the Mira app, not here.
name: Mira

on:
  pull_request:
    types: [opened, reopened, ready_for_review]
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]
  issues:
    types: [opened]

permissions:
  contents: write
  pull-requests: write
  issues: write

concurrency:
  group: mira-${{{{ github.event.pull_request.number || github.event.issue.number }}}}
  cancel-in-progress: false

jobs:
  mira:
    # Comments only start a run when they mention @mira.
    if: >-
      github.event_name == 'pull_request' ||
      contains(github.event.comment.body, '@mira') ||
      contains(github.event.issue.body, '@mira')
    runs-on: ubuntu-latest
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: {action_ref}
        with:
          api-key: ${{{{ secrets.MIRA_API_KEY }}}}
          provider: ${{{{ vars.MIRA_PROVIDER }}}}
          model: ${{{{ vars.MIRA_MODEL }}}}
          base-url: ${{{{ vars.MIRA_BASE_URL }}}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_secrets_open_with_the_private_key() {
        let secret = crypto_box::SecretKey::generate(&mut crypto_box::aead::OsRng);
        let public =
            base64::engine::general_purpose::STANDARD.encode(secret.public_key().as_bytes());
        let sealed = seal(&public, "sk-test").unwrap();
        let raw = base64::engine::general_purpose::STANDARD
            .decode(sealed)
            .unwrap();
        assert_eq!(secret.unseal(&raw).unwrap(), b"sk-test");
    }

    #[test]
    fn the_workflow_uses_secrets_and_variables() {
        let w = workflow("runmira/mira@main");
        assert!(w.contains("uses: runmira/mira@main"));
        assert!(w.contains("api-key: ${{ secrets.MIRA_API_KEY }}"));
        assert!(w.contains("model: ${{ vars.MIRA_MODEL }}"));
        assert!(w.contains("group: mira-${{ github.event.pull_request.number"));
    }
}
