//! Library surface for the `animus-provider-codex` plugin.
//!
//! The binary entrypoint lives in `src/main.rs`. The modules below are
//! exposed so integration tests (and downstream embedders that want to wire
//! the Codex backend without spawning a subprocess) can reach the
//! [`ProviderBackend`](animus_provider_protocol::ProviderBackend)
//! implementation directly.

pub mod backend;
pub mod config;
