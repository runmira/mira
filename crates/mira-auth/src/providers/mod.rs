//! Per-provider OAuth adapters — pure HTTP + URL construction. No
//! framework, no persistence. Callers combine these with the shared
//! [`crate::loopback`] and [`crate::store`] primitives to run a
//! sign-in end-to-end.
//!
//! Each provider module exposes at minimum:
//!
//! * A `PROVIDER` constant — the string key used in
//!   `~/.mira/mira.yaml` and `~/.mira/auth/<provider>.json`.
//! * `authorize_url(...)` — build the URL to hand to the browser.
//! * `exchange_code(...)` — POST the auth-code back to the provider
//!   and receive either an API key ([`openrouter`]) or a full
//!   [`crate::TokenBundle`] ([`openai`]).
//!
//! OpenAI additionally exposes `refresh` because its tokens are
//! short-lived; OpenRouter's aren't, so it doesn't need one.

pub mod openai;
pub mod openrouter;
