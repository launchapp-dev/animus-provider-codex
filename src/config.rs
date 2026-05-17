//! Runtime configuration for the Codex provider plugin.

use anyhow::Result;

/// Configuration sourced from environment variables.
#[derive(Debug, Clone)]
pub struct CodexConfig {
    /// Name (or absolute path) of the Codex CLI binary.
    pub codex_bin: String,
    /// Model identifier to use when the caller does not supply one.
    pub default_model: String,
}

impl CodexConfig {
    /// Read `CODEX_BIN` and `CODEX_DEFAULT_MODEL` from the environment,
    /// falling back to sensible defaults.
    pub fn from_env() -> Result<Self> {
        let codex_bin = std::env::var("CODEX_BIN")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "codex".to_string());
        let default_model = std::env::var("CODEX_DEFAULT_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "gpt-5.2".to_string());

        Ok(Self {
            codex_bin,
            default_model,
        })
    }

    /// Helper for integration tests / embedders that want to construct a
    /// config without going through env vars.
    pub fn for_testing(codex_bin: impl Into<String>) -> Self {
        Self {
            codex_bin: codex_bin.into(),
            default_model: "gpt-5.2".to_string(),
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            codex_bin: "codex".to_string(),
            default_model: "gpt-5.2".to_string(),
        }
    }
}
