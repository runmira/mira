//! The agent loop.
//!
//! [`Session`] owns conversation state, the provider, the tool registry, and
//! the policy. [`Session::send`] runs one user turn to completion, yielding a
//! stream of [`HarnessEvent`]s so a UI can render tokens as they arrive.
//!
//! The loop:
//!
//! 1. Append the user's message to history.
//! 2. Ask the provider for a completion, streaming events out to the caller.
//! 3. If the model requested tool calls: gate each through the policy, then
//!    dispatch, then feed results back and go to step 2.
//! 4. When the model finishes without tool calls, the turn is done.

pub mod approver;
pub mod event;
pub mod goal;
pub mod history;
pub mod persist;
pub mod session;
pub mod verify;

pub use approver::{Approver, AutoApprover};
pub use event::HarnessEvent;
pub use goal::{Evaluation, Goal, GoalStatus, GoalVerdict, DEFAULT_MAX_ITERATIONS};
pub use persist::{FileStore, SessionRecord, SessionStore, StoreError, TurnMeta, UsageTotals};
pub use session::{AutoExtractConfig, Session, SessionConfig};
