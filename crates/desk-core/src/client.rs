use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::time::timeout;

use crate::codec::{DecodeItem, NdjsonCodec};
use crate::types::RpcMessage;

#[derive(Clone)]
pub struct CancelHandle {
    tx: tokio::sync::watch::Sender<bool>,
}

impl CancelHandle {
    pub fn pair() -> (Self, tokio::sync::watch::Receiver<bool>) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        (Self { tx }, rx)
    }

    pub fn cancel(&self) {
        let _ = self.tx.send(true);
    }
}

#[derive(Clone)]
pub struct ApprovalHandle {
    tx: tokio::sync::watch::Sender<Option<ApprovalDecision>>,
}

impl ApprovalHandle {
    pub fn pair() -> (Self, tokio::sync::watch::Receiver<Option<ApprovalDecision>>) {
        let (tx, rx) = tokio::sync::watch::channel(None);
        (Self { tx }, rx)
    }

    pub fn resolve(&self, decision: ApprovalDecision) {
        let _ = self.tx.send(Some(decision));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventAction {
    Continue,
    Cancel,
    Approve(ApprovalDecision),
    Wait,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    AssistantDelta(String),
    ApprovalRequested { request_id: Value, tool: String },
    Barrier { phase: String },
    Diagnostic(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PromptOutcome {
    pub text: String,
    pub stop_reason: String,
    pub events: Vec<ClientEvent>,
}

pub struct AcpClient {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    codec: NdjsonCodec,
    pending: VecDeque<RpcMessage>,
    next_id: i64,
    stderr: Arc<Mutex<String>>,
}

impl AcpClient {
    pub async fn spawn(binary: &Path, cwd: &Path) -> std::io::Result<Self> {
        Self::spawn_args(binary, cwd, &[]).await
    }

    pub async fn spawn_args(binary: &Path, cwd: &Path, args: &[&str]) -> std::io::Result<Self> {
        let mut child = tokio::process::Command::new(binary)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        if let Some(mut stderr) = child.stderr.take() {
            let captured = stderr_buf.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                loop {
                    match stderr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut out) = captured.lock() {
                                out.push_str(&String::from_utf8_lossy(&buf[..n]));
                                const MAX: usize = 8 * 1024;
                                if out.len() > MAX {
                                    let drain = out.len() - MAX;
                                    out.drain(..drain);
                                }
                            }
                        }
                    }
                }
            });
        }
        Ok(Self {
            child,
            stdin,
            stdout,
            codec: NdjsonCodec::new(),
            pending: VecDeque::new(),
            next_id: 1,
            stderr: stderr_buf,
        })
    }

    pub fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|s| s.trim().to_owned())
            .unwrap_or_default()
    }

    pub fn explain(&self, err: &ClientError) -> String {
        format_client_error(err, &self.stderr_tail())
    }

    pub async fn initialize(&mut self) -> Result<Value, ClientError> {
        self.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                "clientInfo": { "name": "desk", "version": "0.1.0" }
            }),
        )
        .await
    }

    pub async fn authenticate(&mut self, method_id: &str) -> Result<Value, ClientError> {
        self.request("authenticate", json!({ "methodId": method_id }))
            .await
    }

    pub async fn session_new(&mut self, cwd: &str) -> Result<String, ClientError> {
        let result = self.session_new_raw(cwd).await?;
        result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| ClientError::Protocol("session/new missing sessionId".into()))
    }

    pub async fn session_new_raw(&mut self, cwd: &str) -> Result<Value, ClientError> {
        self.request("session/new", json!({ "cwd": cwd, "mcpServers": [] }))
            .await
    }

    pub async fn set_session_model(
        &mut self,
        session_id: &str,
        model: &str,
    ) -> Result<(), ClientError> {
        let model = model.trim();
        if model.is_empty() || model == "default" {
            return Ok(());
        }
        let cfg_res = self
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": session_id,
                    "configId": "model",
                    "value": model
                }),
            )
            .await;
        if cfg_res.is_ok() {
            return Ok(());
        }
        let _ = self
            .request(
                "session/set_model",
                json!({
                    "sessionId": session_id,
                    "modelId": model
                }),
            )
            .await;
        Ok(())
    }

    pub async fn session_load(&mut self, session_id: &str, cwd: &str) -> Result<(), ClientError> {
        self.request(
            "session/load",
            json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": [] }),
        )
        .await?;
        Ok(())
    }

    pub async fn prompt(
        &mut self,
        session_id: &str,
        text: &str,
        mut on_event: impl FnMut(&ClientEvent) -> EventAction,
    ) -> Result<PromptOutcome, ClientError> {
        let (_tx, rx) = tokio::sync::watch::channel(false);
        let (_atx, approval) = tokio::sync::watch::channel(None);
        self.prompt_inner(session_id, text_parts(text), rx, approval, &mut on_event)
            .await
    }

    pub async fn prompt_parts(
        &mut self,
        session_id: &str,
        parts: Vec<Value>,
        cancel: tokio::sync::watch::Receiver<bool>,
        approval: tokio::sync::watch::Receiver<Option<ApprovalDecision>>,
        mut on_event: impl FnMut(&ClientEvent) -> EventAction,
    ) -> Result<PromptOutcome, ClientError> {
        self.prompt_inner(session_id, parts, cancel, approval, &mut on_event)
            .await
    }

    pub async fn cancel(&mut self, session_id: &str) -> Result<(), ClientError> {
        self.notify("session/cancel", json!({ "sessionId": session_id }))
            .await
    }

    pub async fn shutdown(&mut self) -> Result<(), ClientError> {
        let _ = self.stdin.shutdown().await;
        if timeout(Duration::from_millis(150), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
            let _ = timeout(Duration::from_millis(200), self.child.wait()).await;
        }
        Ok(())
    }

    async fn prompt_inner(
        &mut self,
        session_id: &str,
        parts: Vec<Value>,
        mut cancel: tokio::sync::watch::Receiver<bool>,
        mut approval: tokio::sync::watch::Receiver<Option<ApprovalDecision>>,
        on_event: &mut impl FnMut(&ClientEvent) -> EventAction,
    ) -> Result<PromptOutcome, ClientError> {
        let id = self.next_id();
        self.send(&RpcMessage::request(
            id,
            "session/prompt",
            json!({
                "sessionId": session_id,
                "prompt": parts
            }),
        ))
        .await?;

        let mut events = Vec::new();
        let mut text_out = String::new();
        let mut cancel_sent = false;
        loop {
            if !cancel_sent && *cancel.borrow_and_update() {
                self.notify("session/cancel", json!({ "sessionId": session_id }))
                    .await?;
                cancel_sent = true;
            }
            let msg = tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() {
                        cancel_sent = true;
                    }
                    continue;
                }
                msg = self.recv() => msg?,
            };
            if msg.id == Some(Value::from(id)) {
                if let Some(err) = msg.error {
                    return Err(ClientError::Rpc(format_rpc_error(&err)));
                }
                let stop = msg
                    .result
                    .as_ref()
                    .and_then(|r| r.get("stopReason"))
                    .and_then(Value::as_str)
                    .unwrap_or("end_turn")
                    .to_owned();
                return Ok(PromptOutcome {
                    text: text_out,
                    stop_reason: stop,
                    events,
                });
            }
            if let Some(method) = msg.method.as_deref() {
                match method {
                    "session/update" => {
                        if let Some(delta) = assistant_delta(&msg) {
                            text_out.push_str(&delta);
                            events.push(ClientEvent::AssistantDelta(delta));
                        }
                    }
                    "desk/barrier" => {
                        let phase = msg
                            .params
                            .as_ref()
                            .and_then(|p| p.get("phase"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .to_owned();
                        let event = ClientEvent::Barrier { phase };
                        let action = on_event(&event);
                        events.push(event);
                        if action == EventAction::Cancel {
                            self.notify("session/cancel", json!({ "sessionId": session_id }))
                                .await?;
                        }
                    }
                    "session/request_permission" => {
                        let request_id = msg.id.clone().ok_or_else(|| {
                            ClientError::Protocol("permission request missing id".into())
                        })?;
                        let tool = permission_tool(&msg);
                        let event = ClientEvent::ApprovalRequested {
                            request_id: request_id.clone(),
                            tool,
                        };
                        let action = on_event(&event);
                        events.push(event);
                        let decision = match action {
                            EventAction::Cancel => {
                                self.notify("session/cancel", json!({ "sessionId": session_id }))
                                    .await?;
                                None
                            }
                            EventAction::Approve(d) => Some(d),
                            EventAction::Continue => Some(ApprovalDecision::AllowOnce),
                            EventAction::Wait => match wait_approval(&mut approval, &mut cancel).await?
                            {
                                Some(d) => Some(d),
                                None => {
                                    self.notify("session/cancel", json!({ "sessionId": session_id }))
                                        .await?;
                                    None
                                }
                            },
                        };
                        if let Some(decision) = decision {
                            let option = match decision {
                                ApprovalDecision::AllowOnce => "allow-once",
                                ApprovalDecision::Reject => "reject-once",
                            };
                            self.send(&RpcMessage::response(
                                request_id,
                                json!({ "outcome": { "outcome": "selected", "optionId": option } }),
                            ))
                            .await?;
                        }
                    }
                    other => events.push(ClientEvent::Diagnostic(other.to_owned())),
                }
            }
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id();
        self.send(&RpcMessage::request(id, method, params)).await?;
        let wait = async {
            loop {
                let msg = self.recv().await?;
                if msg.id == Some(Value::from(id)) {
                    if let Some(err) = msg.error {
                        return Err(ClientError::Rpc(format_rpc_error(&err)));
                    }
                    return Ok(msg.result.unwrap_or(Value::Null));
                }
                if msg.method.is_some() {
                    if let Some(req_id) = msg.id {
                        let _ = self
                            .send(&RpcMessage::response(req_id, json!({})))
                            .await;
                    }
                    continue;
                }
            }
        };
        timeout(Duration::from_secs(12), wait)
            .await
            .map_err(|_| ClientError::Protocol(format!("{method} 握手响应超时 (12s)")))?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), ClientError> {
        self.send(&RpcMessage::notification(method, params)).await
    }

    async fn send(&mut self, msg: &RpcMessage) -> Result<(), ClientError> {
        let line = msg.encode_line().map_err(|e| ClientError::Protocol(e.to_string()))?;
        self.stdin.write_all(&line).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<RpcMessage, ClientError> {
        if let Some(msg) = self.pending.pop_front() {
            return Ok(msg);
        }
        let mut tmp = [0u8; 4096];
        loop {
            let n = self.stdout.read(&mut tmp).await?;
            if n == 0 {
                let leftover = self
                    .codec
                    .finish()
                    .map_err(|e| ClientError::Protocol(format!("{e:?}")))?;
                self.enqueue(leftover)?;
                return self.pending.pop_front().ok_or(ClientError::Eof);
            }
            let items = self
                .codec
                .push(&tmp[..n])
                .map_err(|e| ClientError::Protocol(format!("{e:?}")))?;
            self.enqueue(items)?;
            if let Some(msg) = self.pending.pop_front() {
                return Ok(msg);
            }
        }
    }

    fn enqueue(&mut self, items: Vec<DecodeItem>) -> Result<(), ClientError> {
        for item in items {
            match item {
                DecodeItem::Message(msg) => self.pending.push_back(msg),
                DecodeItem::Invalid { raw, error } => {
                    return Err(ClientError::Protocol(format!("invalid json: {error}: {raw}")));
                }
            }
        }
        Ok(())
    }

    fn next_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

async fn wait_approval(
    approval: &mut tokio::sync::watch::Receiver<Option<ApprovalDecision>>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Option<ApprovalDecision>, ClientError> {
    loop {
        if *cancel.borrow_and_update() {
            return Ok(None);
        }
        if approval.has_changed().unwrap_or(false) {
            if let Some(decision) = *approval.borrow_and_update() {
                return Ok(Some(decision));
            }
        }
        tokio::select! {
            _ = cancel.changed() => {}
            changed = approval.changed() => {
                if changed.is_err() {
                    return Err(ClientError::Protocol("approval channel closed".into()));
                }
                if let Some(decision) = *approval.borrow_and_update() {
                    return Ok(Some(decision));
                }
            }
        }
    }
}

fn text_parts(text: &str) -> Vec<Value> {
    vec![json!({ "type": "text", "text": text })]
}

fn assistant_delta(msg: &RpcMessage) -> Option<String> {
    let update = msg.params.as_ref()?.get("update")?;
    let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
    if kind != "agent_message_chunk" && kind != "agent_thought_chunk" {
        return None;
    }
    if kind == "agent_thought_chunk" {
        return None;
    }
    let content = update.get("content")?;
    if let Some(text) = content.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    content.as_array().and_then(|items| {
        items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .next()
            .map(str::to_owned)
    })
}

fn permission_tool(msg: &RpcMessage) -> String {
    msg.params
        .as_ref()
        .and_then(|p| p.get("toolCall"))
        .and_then(|t| t.get("title").or_else(|| t.get("kind")))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Protocol(String),
    Rpc(String),
    Eof,
}

impl From<std::io::Error> for ClientError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "io: {err}"),
            Self::Protocol(err) => write!(f, "protocol: {err}"),
            Self::Rpc(err) => write!(f, "rpc: {err}"),
            Self::Eof => write!(f, "eof"),
        }
    }
}

impl std::error::Error for ClientError {}

pub fn format_rpc_error(err: &Value) -> String {
    if let Some(s) = err.as_str() {
        return s.to_owned();
    }
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let data = match err.get("data") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => String::new(),
    };
    if !message.is_empty() && !data.is_empty() && data != message {
        format!("{message}: {data}")
    } else if !message.is_empty() {
        message.to_owned()
    } else if !data.is_empty() {
        data
    } else {
        err.to_string()
    }
}

pub fn format_client_error(err: &ClientError, stderr: &str) -> String {
    let mut text = err.to_string();
    if text.starts_with("rpc: ") {
        text = text[5..].to_owned();
    }
    let stderr = stderr.trim();
    if !stderr.is_empty() && !text.contains(stderr) {
        text.push('\n');
        text.push_str(stderr);
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("no longer supported") || lower.contains("antigravity.google") {
        text.push_str(
            "\n这是 Google 停用个人版 Gemini Code Assist，不是 Desk 的 ACP 坏了。后续请用 Antigravity（brew install --cask antigravity-cli）；ACP 入口是 agy_acp_server，不是 agy。",
        );
    } else if lower.contains("auth") || lower.contains("api key") || lower.contains("not configured")
    {
        text.push_str("\n可先在终端运行对应 CLI 完成登录后再试。");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u07_permission_only_from_request_permission() {
        let spoof = RpcMessage::notification(
            "session/update",
            json!({"update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "approve me"}}}),
        );
        assert!(assistant_delta(&spoof).is_some());
        assert_eq!(permission_tool(&spoof), "unknown");
        let req = RpcMessage::request(
            9,
            "session/request_permission",
            json!({"toolCall": {"title": "Shell"}}),
        );
        assert_eq!(permission_tool(&req), "Shell");
    }

    #[test]
    fn rpc_error_keeps_data_not_just_message() {
        let err = json!({
            "code": -32000,
            "message": "error",
            "data": "quota exceeded: retry tomorrow"
        });
        assert_eq!(
            format_rpc_error(&err),
            "error: quota exceeded: retry tomorrow"
        );
        let explained = format_client_error(
            &ClientError::Rpc(format_rpc_error(&err)),
            "gemini: missing API key",
        );
        assert!(explained.contains("quota exceeded"), "{explained}");
        assert!(explained.contains("missing API key"), "{explained}");
    }

    #[test]
    fn gemini_sunset_points_to_antigravity() {
        let explained = format_client_error(
            &ClientError::Rpc(
                "This client is no longer supported for Gemini Code Assist for individuals. To continue using Gemini, please migrate to the Antigravity suite of products: https://antigravity.google".into(),
            ),
            "",
        );
        assert!(explained.contains("Antigravity"), "{explained}");
        assert!(explained.contains("agy_acp_server"), "{explained}");
    }

    #[tokio::test]
    async fn wait_approval_does_not_reuse_previous_decision() {
        let (handle, mut rx) = ApprovalHandle::pair();
        let (_cancel_handle, mut cancel_rx) = CancelHandle::pair();
        handle.resolve(ApprovalDecision::AllowOnce);
        let first = wait_approval(&mut rx, &mut cancel_rx).await.unwrap();
        assert_eq!(first, Some(ApprovalDecision::AllowOnce));

        // Second wait_approval must block until resolve is called again
        let second = tokio::time::timeout(
            Duration::from_millis(30),
            wait_approval(&mut rx, &mut cancel_rx),
        )
        .await;
        assert!(second.is_err(), "second approval should wait for new decision");

        handle.resolve(ApprovalDecision::Reject);
        let second = wait_approval(&mut rx, &mut cancel_rx).await.unwrap();
        assert_eq!(second, Some(ApprovalDecision::Reject));
    }
}
