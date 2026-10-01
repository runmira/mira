//! The unified backend vocabulary — every way a turn can be served.
//!
//! Mira has two halves that can each run a turn: its own harness over
//! an LLM provider, and external agent processes driven over ACP or an
//! app-server protocol. Before this crate the halves shared nothing:
//! providers were a single process-global swap, external agents were a
//! parallel WS channel, and the frontend had to know both vocabularies.
//!
//! The fix is one small set of types both halves speak:
//!
//! * [`id::EngineId`] / [`id::DriverKind`] — the instance/driver split.
//!   An *instance* is one configured backend (the routing key every
//!   selection and frame carries); a *driver* names the implementation.
//!   Both are open slugs — unknown values parse fine and surface as
//!   [`snapshot::EngineState::Unavailable`] rather than failing.
//! * [`selection::ModelSelection`] — `{instance, model, options}`: the
//!   complete answer to "who serves the next turn".
//! * [`instance::EngineInstance`] + [`registry::EngineRegistry`] — what
//!   is configured, derived from `mira.yaml` (native providers + agent
//!   drivers + the user's `engines:` overrides).
//! * [`snapshot::EngineSnapshot`] — health + catalog for one instance,
//!   the shape `GET /api/engines` and the pickers render.
//! * [`posture::Posture`] — the five permission postures, mapped
//!   server-side from harness modes and agent mode ids so no client
//!   ever pattern-matches agent mode names again.
//! * [`native`] — the provider-construction recipe, shared by the
//!   server, the CLI, and the tests.
//!
//! Nothing here holds live sessions or spawns long-lived processes;
//! the only I/O is the external health probe, which callers run in the
//! background and cache.

pub mod external;
pub mod id;
pub mod instance;
pub mod native;
pub mod pool;
pub mod posture;
pub mod registry;
pub mod selection;
pub mod snapshot;

pub use external::driver_config_for;
pub use id::{DriverKind, EngineId};
pub use pool::SwappableProvider;
pub use instance::{instances_from_config, EngineInstance, NATIVE_DRIVER};
pub use posture::{map_postures, Posture, PostureMapping};
pub use registry::EngineRegistry;
pub use selection::ModelSelection;
pub use snapshot::{EngineFlavor, EngineSnapshot, EngineState};
