use animus_plugin_protocol::{PluginInfo, PLUGIN_KIND_PROVIDER};
use animus_plugin_runtime::provider_main_with_capabilities;
use animus_provider_codex::backend::CodexProviderBackend;
use animus_provider_codex::config::CodexConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    emit_manifest_if_requested();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let config = CodexConfig::from_env()?;
    let backend = CodexProviderBackend::new(config);

    let info = PluginInfo {
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        plugin_kind: PLUGIN_KIND_PROVIDER.into(),
        description: Some(env!("CARGO_PKG_DESCRIPTION").into()),
    };

    let extra_capabilities = vec![];

    provider_main_with_capabilities(info, backend, extra_capabilities).await
}

fn emit_manifest_if_requested() {
    if !std::env::args()
        .skip(1)
        .any(|arg| arg == "--manifest" || arg == "-m")
    {
        return;
    }

    let manifest = serde_json::json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "plugin_kind": "provider",
        "description": env!("CARGO_PKG_DESCRIPTION"),
        "protocol_version": animus_plugin_protocol::PROTOCOL_VERSION,
        "capabilities": [
            "agent/run",
            "agent/resume",
            "agent/cancel",
            "health/check"
        ],
        "env_required": [
            {
                "name": "CODEX_BIN",
                "description": "Override the Codex CLI binary path.",
                "required": false
            },
            {
                "name": "CODEX_DEFAULT_MODEL",
                "description": "Fallback model used when the request omits a model.",
                "required": false
            },
            {
                "name": "OPENAI_API_KEY",
                "description": "OpenAI API key forwarded to Codex CLI when API-key auth is configured.",
                "sensitive": true,
                "required": false
            },
            {
                "name": "OPENAI_BASE_URL",
                "description": "Override the OpenAI-compatible API base URL.",
                "required": false
            },
            {
                "name": "OPENAI_ORG",
                "description": "OpenAI organization identifier.",
                "required": false
            },
            {
                "name": "OPENAI_ORG_ID",
                "description": "OpenAI organization id used by some Codex CLI configurations.",
                "required": false
            }
        ]
    });
    println!(
        "{}",
        serde_json::to_string(&manifest).expect("serialize manifest")
    );
    std::process::exit(0);
}
