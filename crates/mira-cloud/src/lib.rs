//! Cloud tasks: start a task, close the laptop, come back to a pull
//! request.
//!
//! The whole Mira session (agent loop included) runs in a cloud sandbox,
//! so nothing depends on the machine that started it:
//!
//! - [`launcher`] (your machine): create the sandbox, get Mira into it,
//!   hand over the [`spec::TaskSpec`] and secrets, start the worker
//!   detached, record the task locally. Returns in seconds.
//! - [`worker`] (the sandbox): clone, open a draft PR, run a headless
//!   `/goal` session, push as it goes, summarize, mark ready.
//! - [`store`]: the local list behind `mira cloud list`.

use thiserror::Error;

pub mod git;
pub mod github;
pub mod launcher;
pub mod setup;
pub mod spec;
pub mod store;
pub mod worker;

#[derive(Debug, Error)]
pub enum CloudError {
    #[error("configuration: {0}")]
    Config(String),
    #[error("git: {0}")]
    Git(String),
    #[error("github: {0}")]
    GitHub(String),
    #[error("sandbox: {0}")]
    Sandbox(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<mira_compute::ComputeError> for CloudError {
    fn from(e: mira_compute::ComputeError) -> Self {
        CloudError::Sandbox(e.to_string())
    }
}
