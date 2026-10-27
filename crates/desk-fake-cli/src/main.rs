use std::io::{self, BufRead, Write};

use serde_json::{json, Value};

fn main() {
    let scenario = scenario_from_args();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut session_id = String::from("fake-session");
    let mut next_session = 1u32;
    let mut pending_prompt: Option<Value> = None;
    let mut pending_text = String::new();
    let mut current_mode = scenario.clone();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();

        match method {
            "initialize" => reply(
                &mut stdout,
                id,
                json!({
                    "protocolVersion": 1,
                    "agentCapabilities": { "loadSession": true }
                }),
            ),
            "authenticate" => reply(&mut stdout, id, json!({})),
            "session/new" => {
                session_id = format!("fake-{}-{next_session}", std::process::id());
                next_session += 1;
                reply(
                    &mut stdout,
                    id,
                    json!({
                        "sessionId": session_id,
                        "configOptions": [{
                            "id": "model",
                            "category": "model",
                            "currentValue": "fake-default",
                            "options": [
                                { "value": "fake-default", "name": "Fake Default" },
                                { "value": "fake-fast", "name": "Fake Fast" }
                            ]
                        }],
                        "models": {
                            "currentModelId": "fake-default",
                            "availableModels": [
                                { "modelId": "fake-default", "name": "Fake Default" },
                                { "modelId": "fake-fast", "name": "Fake Fast" }
                            ]
                        }
                    }),
                );
            }
            "session/set_config_option" | "session/set_model" => {
                reply(&mut stdout, id, json!({}));
            }
            "session/load" => {
                if let Some(sid) = msg.pointer("/params/sessionId").and_then(Value::as_str) {
                    if sid.starts_with("missing-") {
                        reply_error(
                            &mut stdout,
                            id,
                            json!({
                                "code": -32602,
                                "message": "Invalid params",
                                "data": { "message": format!("Session \"{sid}\" not found") }
                            }),
                        );
                        continue;
                    }
                    session_id = sid.to_owned();
                }
                reply(&mut stdout, id, json!({ "sessionId": session_id }));
            }
            "session/prompt" => {
                if let Some(sid) = msg.pointer("/params/sessionId").and_then(Value::as_str) {
                    if sid.starts_with("missing-") {
                        reply_error(
                            &mut stdout,
                            id,
                            json!({
                                "code": -32602,
                                "message": "Invalid params",
                                "data": { "message": format!("Session \"{sid}\" not found") }
                            }),
                        );
                        continue;
                    }
                }
                let text = prompt_text(&msg).unwrap_or_default();
                pending_text = text.clone();
                pending_prompt = id.clone();
                current_mode = if text == "HANG" {
                    "hang".into()
                } else if text == "APPROVE" {
                    "approve".into()
                } else if text == "FAIL" {
                    "fail".into()
                } else {
                    scenario.clone()
                };
                match current_mode.as_str() {
                    "approve" => {
                        notify(
                            &mut stdout,
                            "desk/barrier",
                            json!({ "phase": "awaiting_permission", "sessionId": session_id }),
                        );
                        request(
                            &mut stdout,
                            1001,
                            "session/request_permission",
                            json!({
                                "sessionId": session_id,
                                "toolCall": { "kind": "execute", "title": "uname" },
                                "promptText": text
                            }),
                        );
                    }
                    "hang" => {
                        notify(
                            &mut stdout,
                            "desk/barrier",
                            json!({ "phase": "hanging", "sessionId": session_id }),
                        );
                    }
                    "fail" => {
                        eprintln!("gemini stderr: missing API key");
                        reply_error(
                            &mut stdout,
                            pending_prompt.take(),
                            json!({
                                "code": -32000,
                                "message": "error",
                                "data": "quota exceeded: retry tomorrow"
                            }),
                        );
                    }
                    _ => {
                        let reply_text = if let Some(contents) = read_prompt_files(&msg) {
                            format!("file:{contents}")
                        } else if text == "RECALL" {
                            format!("echo:{}", load_memory(&session_id))
                        } else {
                            save_memory(&session_id, &text);
                            format!("echo:{text}")
                        };
                        notify(
                            &mut stdout,
                            "session/update",
                            json!({
                                "sessionId": session_id,
                                "update": {
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": { "type": "text", "text": reply_text }
                                }
                            }),
                        );
                        reply(&mut stdout, pending_prompt.take(), json!({ "stopReason": "end_turn" }));
                    }
                }
            }
            "session/cancel" => {
                notify(
                    &mut stdout,
                    "session/update",
                    json!({
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": "cancelled" }
                        }
                    }),
                );
                reply(
                    &mut stdout,
                    pending_prompt.take(),
                    json!({ "stopReason": "cancelled" }),
                );
            }
            _ => {
                if pending_prompt.is_some() && current_mode == "approve" && is_permission_response(&msg) {
                    let option = msg
                        .pointer("/result/outcome/optionId")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let body = if option == "allow-once" {
                        format!("approved:{pending_text}")
                    } else {
                        format!("rejected:{pending_text}")
                    };
                    notify(
                        &mut stdout,
                        "session/update",
                        json!({
                            "update": {
                                "sessionUpdate": "agent_message_chunk",
                                "content": { "type": "text", "text": body }
                            }
                        }),
                    );
                    reply(
                        &mut stdout,
                        pending_prompt.take(),
                        json!({ "stopReason": "end_turn" }),
                    );
                }
            }
        }
    }
}

fn is_permission_response(msg: &Value) -> bool {
    msg.get("id") == Some(&json!(1001)) && msg.get("result").is_some()
}

fn scenario_from_args() -> String {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--scenario" {
            return args.next().unwrap_or_else(|| "reply".into());
        }
    }
    std::env::var("DESK_FAKE_SCENARIO").unwrap_or_else(|_| "reply".into())
}

fn read_prompt_files(msg: &Value) -> Option<String> {
    let items = msg.pointer("/params/prompt")?.as_array()?;
    let mut out = String::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("resource") => {
                if let Some(text) = item.pointer("/resource/text").and_then(Value::as_str) {
                    out.push_str(text);
                    continue;
                }
                if let Some(text) = read_file_uri(
                    item.pointer("/resource/uri")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("uri").and_then(Value::as_str)),
                ) {
                    out.push_str(&text);
                }
            }
            Some("resource_link") => {
                if let Some(text) = read_file_uri(item.get("uri").and_then(Value::as_str)) {
                    out.push_str(&text);
                }
            }
            Some("image") => {
                let n = item.get("data").and_then(Value::as_str).map(|d| d.len()).unwrap_or(0);
                out.push_str(&format!("image:{n}"));
            }
            _ => {}
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn read_file_uri(uri: Option<&str>) -> Option<String> {
    let path = uri?.strip_prefix("file://")?;
    std::fs::read_to_string(path).ok()
}

fn prompt_text(msg: &Value) -> Option<String> {
    let items = msg.pointer("/params/prompt")?.as_array()?;
    let mut out = String::new();
    for item in items {
        if let Some(text) = item.get("text").and_then(Value::as_str) {
            out.push_str(text);
        }
    }
    Some(out)
}

fn memory_path(session_id: &str) -> std::path::PathBuf {
    let dir = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(".desk-fake");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(session_id)
}

fn save_memory(session_id: &str, text: &str) {
    let _ = std::fs::write(memory_path(session_id), text);
}

fn load_memory(session_id: &str) -> String {
    std::fs::read_to_string(memory_path(session_id)).unwrap_or_default()
}

fn reply(stdout: &mut io::Stdout, id: Option<Value>, result: Value) {
    let Some(id) = id else { return };
    write_json(stdout, json!({"jsonrpc":"2.0","id": id, "result": result}));
}

fn reply_error(stdout: &mut io::Stdout, id: Option<Value>, error: Value) {
    let Some(id) = id else { return };
    write_json(stdout, json!({"jsonrpc":"2.0","id": id, "error": error}));
}

fn request(stdout: &mut io::Stdout, id: i64, method: &str, params: Value) {
    write_json(
        stdout,
        json!({"jsonrpc":"2.0","id": id, "method": method, "params": params}),
    );
}

fn notify(stdout: &mut io::Stdout, method: &str, params: Value) {
    write_json(
        stdout,
        json!({"jsonrpc":"2.0","method": method, "params": params}),
    );
}

fn write_json(stdout: &mut io::Stdout, value: Value) {
    writeln!(stdout, "{value}").ok();
    stdout.flush().ok();
}
