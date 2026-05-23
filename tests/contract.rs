use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use animus_plugin_protocol::HealthStatus;
use animus_provider_codex::backend::CodexProviderBackend;
use animus_provider_codex::config::CodexConfig;
use animus_provider_protocol::{
    AgentNotification, AgentRunRequest, NotificationSink, ProviderBackend,
};
use animus_session_backend::{
    Result as SessionResult, SessionBackend, SessionBackendInfo, SessionBackendKind,
    SessionCapabilities, SessionEvent, SessionRequest, SessionRun, SessionStability,
};
use async_trait::async_trait;
use tokio::sync::mpsc;

/// Programmable fake session backend used for the contract tests.
///
/// `events` is the canned event stream returned from `start_session` /
/// `resume_session`. `session_id` is what the backend reports back to the
/// caller. `terminated` records which session ids were passed to
/// `terminate_session` so tests can assert cancellation.
struct FakeSession {
    events: Mutex<Option<Vec<SessionEvent>>>,
    session_id: Option<String>,
    terminated: Arc<Mutex<Vec<String>>>,
}

impl FakeSession {
    fn new(
        events: Vec<SessionEvent>,
        session_id: Option<String>,
    ) -> (Self, Arc<Mutex<Vec<String>>>) {
        let terminated = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                events: Mutex::new(Some(events)),
                session_id,
                terminated: terminated.clone(),
            },
            terminated,
        )
    }

    fn drain_events(&self) -> Vec<SessionEvent> {
        self.events.lock().unwrap().take().unwrap_or_default()
    }

    fn make_run(&self) -> SessionRun {
        let (tx, rx) = mpsc::channel(16);
        for event in self.drain_events() {
            // Send synchronously through `try_send` — channel is sized for it.
            tx.try_send(event).expect("fake session channel");
        }
        drop(tx);
        SessionRun {
            session_id: self.session_id.clone(),
            events: rx,
            selected_backend: "fake-codex".to_string(),
            fallback_reason: None,
            pid: None,
        }
    }
}

#[async_trait]
impl SessionBackend for FakeSession {
    fn info(&self) -> SessionBackendInfo {
        SessionBackendInfo {
            kind: SessionBackendKind::CodexSdk,
            provider_tool: "codex".to_string(),
            stability: SessionStability::Experimental,
            display_name: "Fake Codex".to_string(),
        }
    }

    fn capabilities(&self) -> SessionCapabilities {
        SessionCapabilities {
            supports_resume: true,
            supports_terminate: true,
            supports_permissions: true,
            supports_mcp: true,
            supports_tool_events: false,
            supports_thinking_events: true,
            supports_artifact_events: false,
            supports_usage_metadata: true,
        }
    }

    async fn start_session(&self, _request: SessionRequest) -> SessionResult<SessionRun> {
        Ok(self.make_run())
    }

    async fn resume_session(
        &self,
        _request: SessionRequest,
        _session_id: &str,
    ) -> SessionResult<SessionRun> {
        Ok(self.make_run())
    }

    async fn terminate_session(&self, session_id: &str) -> SessionResult<()> {
        self.terminated.lock().unwrap().push(session_id.to_string());
        Ok(())
    }
}

fn run_request(model: Option<&str>, prompt: &str) -> AgentRunRequest {
    AgentRunRequest {
        session_id: None,
        prompt: prompt.to_string(),
        model: model.map(|s| s.to_string()),
        system_prompt: None,
        cwd: PathBuf::from("/tmp"),
        project_root: None,
        permission_mode: None,
        timeout_secs: None,
        env: HashMap::new(),
        mcp_servers: None,
        tools: None,
        response_schema: None,
        runtime_contract: None,
        extras: HashMap::new(),
    }
}

fn resume_request(session_id: &str, prompt: &str) -> AgentRunRequest {
    AgentRunRequest {
        session_id: Some(session_id.to_string()),
        ..run_request(Some("gpt-5.2"), prompt)
    }
}

#[tokio::test]
async fn run_agent_via_fake_session() {
    let events = vec![
        SessionEvent::Started {
            backend: "fake-codex".to_string(),
            session_id: Some("sess-1".to_string()),
            pid: Some(42),
        },
        SessionEvent::TextDelta {
            text: "hello".to_string(),
        },
        SessionEvent::TextDelta {
            text: " world".to_string(),
        },
        SessionEvent::Finished { exit_code: Some(0) },
    ];
    let (fake, _terminated) = FakeSession::new(events, Some("sess-1".to_string()));
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    let response = backend
        .run_agent(run_request(Some("gpt-5.2"), "ping"))
        .await
        .expect("run_agent should succeed");

    assert_eq!(response.output, "hello world");
    assert_eq!(response.session_id, "sess-1");
    assert_eq!(response.exit_code, 0);
    assert!(response.backend.contains("gpt-5.2"));
}

#[tokio::test]
async fn run_agent_prefers_final_text_over_deltas() {
    let events = vec![
        SessionEvent::TextDelta {
            text: "partial".to_string(),
        },
        SessionEvent::FinalText {
            text: "FINAL".to_string(),
        },
        SessionEvent::Finished { exit_code: Some(0) },
    ];
    let (fake, _terminated) = FakeSession::new(events, Some("sess-2".to_string()));
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    let response = backend
        .run_agent(run_request(Some("gpt-5.2"), "ping"))
        .await
        .expect("run_agent should succeed");

    assert_eq!(response.output, "FINAL");
}

#[tokio::test]
async fn resume_agent_via_fake_session() {
    let events = vec![
        SessionEvent::TextDelta {
            text: "resumed:".to_string(),
        },
        SessionEvent::TextDelta {
            text: "ok".to_string(),
        },
        SessionEvent::Finished { exit_code: Some(0) },
    ];
    let (fake, _terminated) = FakeSession::new(events, Some("sess-resume".to_string()));
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    let response = backend
        .resume_agent(resume_request("sess-resume", "continue"))
        .await
        .expect("resume_agent should succeed");

    assert_eq!(response.output, "resumed:ok");
    assert_eq!(response.session_id, "sess-resume");
}

#[tokio::test]
async fn resume_agent_requires_session_id() {
    let (fake, _terminated) = FakeSession::new(Vec::new(), None);
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    let err = backend
        .resume_agent(run_request(Some("gpt-5.2"), "no session"))
        .await
        .expect_err("resume without session id should fail");
    assert!(format!("{err}").contains("session_id"));
}

#[tokio::test]
async fn cancel_agent_forwards_to_session() {
    let (fake, terminated) = FakeSession::new(Vec::new(), None);
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    backend
        .cancel_agent("sess-cancel")
        .await
        .expect("cancel should succeed");

    let log = terminated.lock().unwrap();
    assert_eq!(log.as_slice(), &["sess-cancel".to_string()]);
}

#[tokio::test]
async fn health_unhealthy_when_codex_bin_missing() {
    let backend = CodexProviderBackend::new(CodexConfig::for_testing(
        "/definitely/does/not/exist/codex-xyz",
    ));
    let health = backend.health().await.expect("health should not error");
    assert_eq!(health.status, HealthStatus::Unhealthy);
    assert!(health.last_error.is_some());
}

#[tokio::test]
async fn health_healthy_when_codex_bin_on_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stub = dir.path().join("codex-stub");
    std::fs::write(&stub, "#!/bin/sh\nexit 0\n").expect("write stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&stub).unwrap().permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&stub, perm).unwrap();
    }

    let original_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{}", dir.path().display(), original_path);
    // SAFETY: tests in this binary run on a single tokio runtime and we
    // restore PATH before returning, but cargo test still parallelises across
    // binaries. PATH is process-wide; rely on the unique stub name to avoid
    // collisions.
    std::env::set_var("PATH", &new_path);

    let backend = CodexProviderBackend::new(CodexConfig::for_testing(
        stub.file_name().unwrap().to_str().unwrap(),
    ));
    let health = backend.health().await.expect("health should not error");
    std::env::set_var("PATH", original_path);

    assert_eq!(health.status, HealthStatus::Healthy);
}

#[tokio::test]
async fn run_agent_streaming_emits_notifications_per_event() {
    let events = vec![
        SessionEvent::Started {
            backend: "fake-codex".to_string(),
            session_id: Some("sess-stream".to_string()),
            pid: Some(7),
        },
        SessionEvent::TextDelta {
            text: "hel".to_string(),
        },
        SessionEvent::Thinking {
            text: "considering...".to_string(),
        },
        SessionEvent::TextDelta {
            text: "lo".to_string(),
        },
        SessionEvent::ToolCall {
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({"cmd": "ls"}),
            server: None,
        },
        SessionEvent::ToolResult {
            tool_name: "shell".to_string(),
            output: serde_json::json!({"stdout": "a\nb"}),
            success: true,
        },
        SessionEvent::Error {
            message: "transient".to_string(),
            recoverable: true,
        },
        SessionEvent::FinalText {
            text: "hello".to_string(),
        },
        SessionEvent::Finished { exit_code: Some(0) },
    ];
    let (fake, _terminated) = FakeSession::new(events, Some("sess-stream".to_string()));
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));

    let recorded: Arc<Mutex<Vec<AgentNotification>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_clone = recorded.clone();
    let sink = NotificationSink::new(move |n| recorded_clone.lock().unwrap().push(n));

    let response = backend
        .run_agent_streaming(run_request(Some("gpt-5.2"), "ping"), sink)
        .await
        .expect("streaming run should succeed");

    assert_eq!(response.output, "hello");
    assert_eq!(response.session_id, "sess-stream");

    let log = recorded.lock().unwrap().clone();
    let kinds: Vec<&str> = log.iter().map(|n| n.method()).collect();
    assert!(kinds.contains(&"agent/output"));
    assert!(kinds.contains(&"agent/thinking"));
    assert!(kinds.contains(&"agent/toolCall"));
    assert!(kinds.contains(&"agent/toolResult"));
    assert!(kinds.contains(&"agent/error"));

    let output_count = log
        .iter()
        .filter(|n| matches!(n, AgentNotification::Output { .. }))
        .count();
    assert_eq!(output_count, 3, "expected 2 deltas + 1 final");

    let final_flagged = log
        .iter()
        .filter(|n| matches!(n, AgentNotification::Output { is_final: true, .. }))
        .count();
    assert_eq!(final_flagged, 1);

    for n in &log {
        match n {
            AgentNotification::Output { session_id, .. }
            | AgentNotification::Thinking { session_id, .. }
            | AgentNotification::ToolCall { session_id, .. }
            | AgentNotification::ToolResult { session_id, .. }
            | AgentNotification::Error { session_id, .. } => {
                assert_eq!(session_id, "sess-stream");
            }
        }
    }
}

#[tokio::test]
async fn run_agent_streaming_matches_run_agent_output() {
    let events = vec![
        SessionEvent::Started {
            backend: "fake-codex".to_string(),
            session_id: Some("sess-eq".to_string()),
            pid: None,
        },
        SessionEvent::TextDelta {
            text: "foo".to_string(),
        },
        SessionEvent::TextDelta {
            text: "bar".to_string(),
        },
        SessionEvent::Finished { exit_code: Some(0) },
    ];
    let (fake_a, _) = FakeSession::new(events.clone(), Some("sess-eq".to_string()));
    let backend_a = CodexProviderBackend::with_session(fake_a, CodexConfig::for_testing("codex"));
    let non_streaming = backend_a
        .run_agent(run_request(Some("gpt-5.2"), "ping"))
        .await
        .expect("run_agent");

    let (fake_b, _) = FakeSession::new(events, Some("sess-eq".to_string()));
    let backend_b = CodexProviderBackend::with_session(fake_b, CodexConfig::for_testing("codex"));
    let streaming = backend_b
        .run_agent_streaming(
            run_request(Some("gpt-5.2"), "ping"),
            NotificationSink::noop(),
        )
        .await
        .expect("run_agent_streaming");

    assert_eq!(streaming.output, non_streaming.output);
    assert_eq!(streaming.session_id, non_streaming.session_id);
    assert_eq!(streaming.exit_code, non_streaming.exit_code);
}

#[tokio::test]
async fn manifest_advertises_resume_and_cancellation() {
    let (fake, _terminated) = FakeSession::new(Vec::new(), None);
    let backend = CodexProviderBackend::with_session(fake, CodexConfig::for_testing("codex"));
    let manifest = backend.manifest();
    assert_eq!(manifest.tool, "codex");
    assert!(manifest.capabilities.resume);
    assert!(manifest.capabilities.cancellation);
    assert!(manifest.capabilities.streaming);
    assert!(manifest.capabilities.write_capable);
    assert!(manifest.supported_models.iter().any(|m| m == "gpt-5.2"));
}
