//! [`ProviderBackend`] implementation that fronts a Codex
//! [`SessionBackend`].

use std::sync::Arc;
use std::time::Instant;

use animus_plugin_protocol::{HealthCheckResult, HealthStatus};
use animus_provider_protocol::{
    AgentResumeRequest, AgentRunRequest, AgentRunResponse, BackendError, ProviderBackend,
    ProviderCapabilities, ProviderManifest,
};
use animus_session_backend::{
    lookup_binary_in_path, CodexSessionBackend, SessionBackend, SessionEvent, SessionRequest,
    SessionRun,
};
use async_trait::async_trait;

use crate::config::CodexConfig;

/// Provider backend that translates Animus `agent/*` calls into Codex
/// session calls.
pub struct CodexProviderBackend {
    session: Arc<dyn SessionBackend>,
    config: CodexConfig,
}

impl CodexProviderBackend {
    /// Build a backend that drives the real [`CodexSessionBackend`].
    pub fn new(config: CodexConfig) -> Self {
        Self {
            session: Arc::new(CodexSessionBackend::new()),
            config,
        }
    }

    /// Inject a custom [`SessionBackend`] (useful for tests).
    pub fn with_session<S>(session: S, config: CodexConfig) -> Self
    where
        S: SessionBackend + 'static,
    {
        Self {
            session: Arc::new(session),
            config,
        }
    }

    fn build_session_request(&self, request: &AgentRunRequest) -> SessionRequest {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| self.config.default_model.clone());

        let env_vars: Vec<(String, String)> = request
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let mut extras = serde_json::Map::new();
        if let Some(system) = &request.system_prompt {
            extras.insert("system_prompt".to_string(), system.clone().into());
        }
        if let Some(tools) = &request.tools {
            extras.insert("tools".to_string(), tools.clone());
        }
        if let Some(schema) = &request.response_schema {
            extras.insert("response_schema".to_string(), schema.clone());
        }
        if let Some(contract) = &request.runtime_contract {
            extras.insert("runtime_contract".to_string(), contract.clone());
        }
        for (key, value) in &request.extras {
            extras.entry(key.clone()).or_insert(value.clone());
        }

        SessionRequest {
            tool: "codex".to_string(),
            model,
            prompt: request.prompt.clone(),
            cwd: request.cwd.clone(),
            project_root: request.project_root.clone(),
            mcp_endpoint: None,
            permission_mode: request.permission_mode.clone(),
            timeout_secs: request.timeout_secs,
            env_vars,
            extras: serde_json::Value::Object(extras),
        }
    }
}

#[async_trait]
impl ProviderBackend for CodexProviderBackend {
    fn manifest(&self) -> ProviderManifest {
        let caps = self.session.capabilities();
        ProviderManifest {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            description: env!("CARGO_PKG_DESCRIPTION").to_string(),
            supported_models: vec![
                "gpt-5.2".to_string(),
                "gpt-5.1".to_string(),
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "gpt-5-codex".to_string(),
            ],
            tool: "codex".to_string(),
            capabilities: ProviderCapabilities {
                streaming: true,
                resume: caps.supports_resume,
                cancellation: caps.supports_terminate,
                write_capable: true,
                mcp: caps.supports_mcp,
            },
        }
    }

    async fn run_agent(&self, request: AgentRunRequest) -> Result<AgentRunResponse, BackendError> {
        let started = Instant::now();
        let session_request = self.build_session_request(&request);
        let model_label = session_request.model.clone();

        let run = self
            .session
            .start_session(session_request)
            .await
            .map_err(map_session_error)?;

        Ok(drain_session(run, started, model_label).await)
    }

    async fn resume_agent(
        &self,
        request: AgentResumeRequest,
    ) -> Result<AgentRunResponse, BackendError> {
        let session_id = request.session_id.clone().ok_or_else(|| {
            BackendError::Other(anyhow::anyhow!(
                "codex: resume requires session_id on the request"
            ))
        })?;
        let started = Instant::now();
        let session_request = self.build_session_request(&request);
        let model_label = session_request.model.clone();

        let run = self
            .session
            .resume_session(session_request, &session_id)
            .await
            .map_err(map_session_error)?;

        Ok(drain_session(run, started, model_label).await)
    }

    async fn cancel_agent(&self, session_id: &str) -> Result<(), BackendError> {
        self.session
            .terminate_session(session_id)
            .await
            .map_err(|e| BackendError::Other(anyhow::anyhow!("codex cancel: {e}")))
    }

    async fn health(&self) -> Result<HealthCheckResult, BackendError> {
        match lookup_binary_in_path(&self.config.codex_bin) {
            Some(path) => Ok(HealthCheckResult {
                status: HealthStatus::Healthy,
                uptime_ms: None,
                memory_usage_bytes: None,
                last_error: Some(format!("codex binary resolved at {}", path.display())),
            }),
            None => Ok(HealthCheckResult {
                status: HealthStatus::Unhealthy,
                uptime_ms: None,
                memory_usage_bytes: None,
                last_error: Some(format!(
                    "codex binary `{}` not found on PATH",
                    self.config.codex_bin
                )),
            }),
        }
    }
}

fn map_session_error(error: animus_session_backend::Error) -> BackendError {
    use animus_session_backend::Error as SessionError;
    match error {
        SessionError::CliNotFound(msg) => {
            BackendError::Unavailable(format!("codex cli not found: {msg}"))
        }
        SessionError::ValidationFailed(msg) => {
            BackendError::Other(anyhow::anyhow!("codex validation failed: {msg}"))
        }
        SessionError::ExecutionFailed(msg) => BackendError::RunFailed(msg),
        SessionError::IoError(err) => BackendError::SessionStartFailed(err.to_string()),
        SessionError::SerializationError(msg) => {
            BackendError::Other(anyhow::anyhow!("codex serialization: {msg}"))
        }
        SessionError::Other(err) => BackendError::Other(err),
    }
}

async fn drain_session(
    mut run: SessionRun,
    started: Instant,
    model_label: String,
) -> AgentRunResponse {
    let selected_backend = run.selected_backend.clone();
    let mut session_id = run.session_id.clone();
    let mut output = String::new();
    let mut final_text: Option<String> = None;
    let mut tool_calls: Vec<serde_json::Value> = Vec::new();
    let mut tool_results: Vec<serde_json::Value> = Vec::new();
    let mut thinking: Vec<String> = Vec::new();
    let mut metadata: Vec<serde_json::Value> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut exit_code: i32 = 0;

    while let Some(event) = run.events.recv().await {
        match event {
            SessionEvent::Started {
                session_id: sid, ..
            } => {
                if session_id.is_none() {
                    session_id = sid;
                }
            }
            SessionEvent::TextDelta { text } => {
                output.push_str(&text);
            }
            SessionEvent::FinalText { text } => {
                final_text = Some(text);
            }
            SessionEvent::ToolCall {
                tool_name,
                arguments,
                server,
            } => {
                tool_calls.push(serde_json::json!({
                    "tool_name": tool_name,
                    "arguments": arguments,
                    "server": server,
                }));
            }
            SessionEvent::ToolResult {
                tool_name,
                output: tool_output,
                success,
            } => {
                tool_results.push(serde_json::json!({
                    "tool_name": tool_name,
                    "output": tool_output,
                    "success": success,
                }));
            }
            SessionEvent::Thinking { text } => {
                thinking.push(text);
            }
            SessionEvent::Artifact {
                artifact_id,
                metadata: meta,
            } => {
                metadata.push(serde_json::json!({
                    "artifact_id": artifact_id,
                    "metadata": meta,
                }));
            }
            SessionEvent::Metadata { metadata: meta } => {
                metadata.push(meta);
            }
            SessionEvent::Error {
                message,
                recoverable,
            } => {
                errors.push(message);
                if !recoverable {
                    exit_code = 1;
                }
            }
            SessionEvent::Finished { exit_code: code } => {
                if let Some(c) = code {
                    exit_code = c;
                }
                break;
            }
        }
    }

    if let Some(text) = final_text {
        if !text.is_empty() {
            output = text;
        }
    }

    let backend_label = if selected_backend.is_empty() {
        format!("codex:{model_label}")
    } else {
        format!("{selected_backend}:{model_label}")
    };

    AgentRunResponse {
        session_id: session_id.unwrap_or_default(),
        exit_code,
        output,
        metadata,
        tool_calls,
        tool_results,
        thinking,
        errors,
        duration_ms: started.elapsed().as_millis() as u64,
        backend: backend_label,
        tokens_used: None,
        decision_verdict: None,
    }
}
