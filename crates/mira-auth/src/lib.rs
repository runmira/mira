//! Shared OAuth (PKCE) sign-in machinery for Mira.
//!
//! `mira-server` and `mira-cli` both need to drive the same
//! browser-loopback OAuth flow (OpenRouter, "Sign in with ChatGPT").
//! The server wraps it in axum handlers so a browser can kick it off;
//! the CLI drives it directly from `mira login <provider>`. Everything
//! that isn't specific to *how* the flow is triggered lives here:
//!
//! * [`pkce`]      — RFC 7636 verifier/challenge/state generation.
//! * [`store`]     — atomic per-provider JSON token bundles under
//!   `~/.mira/auth/<provider>.json`, mode 0600 on Unix.
//! * [`loopback`]  — bind a `127.0.0.1` [`TcpListener`], serve one HTTP
//!   response, extract the OAuth callback query. The server's OpenAI
//!   flow used to inline this; the CLI needs the same primitives.
//! * [`providers`] — pure request/response adapters for each provider
//!   (`openrouter`, `openai`) — hit the OAuth token endpoints, return
//!   plain [`String`] / [`TokenBundle`], no framework in sight.
//!
//! What deliberately isn't here:
//!
//! * The axum handlers themselves (`mira-server/src/oauth/*.rs` keeps
//!   those; they now call into this crate for the mechanics).
//! * The live-provider "hot-swap" — that's server state, not shared.
//! * Mirroring the freshly-minted API key into `mira.yaml` — [`config`]
//!   below has a tiny helper for it, so both server and CLI use the
//!   same code path, but the crate doesn't force it on callers.

pub mod config;
pub mod loopback;
pub mod pkce;
pub mod providers;
pub mod store;

pub use store::{load, save, token_path, token_path_at, TokenBundle};

/// Convenience re-export so callers don't have to depend on
/// `tokio::net` for the loopback types.
pub use tokio::net::TcpListener;
