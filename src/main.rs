use animus_plugin_protocol::{PluginInfo, PLUGIN_KIND_PROVIDER};
use animus_plugin_runtime::provider_main;
use animus_provider_codex::backend::CodexProviderBackend;
use animus_provider_codex::config::CodexConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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

    provider_main(info, backend).await
}
