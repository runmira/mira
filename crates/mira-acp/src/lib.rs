//! Agent Client Protocol runtime.
//!
//! ACP is the protocol Zed introduced and that Grok, Cursor and Google's
//! Antigravity all speak, so one implementation covers all three. The split
//! mirrors the separation that makes multi-agent hosting tractable:
//!
//! * [`driver`] / [`drivers`] — the per-vendor layer: which binary, which
//!   subcommand, which flag means "auto-approve", which auth method id.
//!   Pure data, no I/O, fully unit-tested without an agent installed.
//! * [`framing`] — newline-delimited JSON-RPC over a child's stdio, with
//!   the demultiplexing the bidirectional protocol requires.
//! * [`conn`] — the bidirectional JSON-RPC connection, which has to
//!   demultiplex one id space shared by both directions.
//! * [`events`] — the only place that speaks ACP's dialect; everything
//!   above it sees normalized, provenance-tagged events.
//! * [`host`] — the capabilities an agent may ask of us, declared as ports
//!   so `mira-server` can bind them to Mira's sandbox, policy and approval
//!   machinery without this crate depending on any of it.
//!
//! Nothing here spawns a process yet, and nothing depends on the rest of
//! Mira, so each layer can be verified on its own.

pub mod agent_sessions;
pub mod appserver;
pub mod conn;
pub mod driver;
pub mod drivers;
pub mod events;
pub mod framing;
pub mod host;
pub mod native;
pub mod process;
pub mod session;
pub mod snapshot;
pub mod status;
pub mod which;

pub use conn::{AgentCallback, CallOutcome, ConnError, Connection, NullCallbacks};
pub use host::PendingPermissions;
pub use host::{AcpHost, DenyAll, EventPort, FilePort, HostError, PermissionPort, TerminalPort};
pub use process::{AcpAgent, AgentProcess, SpawnError, StartError, StartSpec};
pub use session::{AcpSession, ClientCaps, SessionError};
pub use status::{probe, AgentState, AgentStatus};
