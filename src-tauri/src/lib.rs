use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use desk_core::{App, ApprovalDecision, Provider, Snapshot};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex;

#[derive(Clone, Serialize)]
struct Jump {
    session_id: String,
    notification_id: String,
    run_id: Option<String>,
}

pub struct Desk(pub Mutex<App>);

#[tauri::command]
async fn snapshot(state: tauri::State<'_, Arc<Desk>>) -> Result<Snapshot, String> {
    state.0.lock().await.snapshot()
}

#[tauri::command]
async fn pump(state: tauri::State<'_, Arc<Desk>>) -> Result<Snapshot, String> {
    let mut app = state.0.lock().await;
    app.pump().await?;
    app.snapshot()
}

#[tauri::command]
async fn list_messages(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
    limit: Option<u32>,
    before_created_at: Option<i64>,
    before_id: Option<String>,
) -> Result<Vec<desk_core::Message>, String> {
    let app = state.0.lock().await;
    match limit {
        Some(n) => app.list_messages_page(
            &session_id,
            before_created_at,
            before_id.as_deref(),
            n as usize,
        ),
        None => app.list_messages(&session_id),
    }
}

#[tauri::command]
async fn create_team(
    state: tauri::State<'_, Arc<Desk>>,
    name: String,
) -> Result<desk_core::Team, String> {
    state.0.lock().await.create_team(&name)
}

#[tauri::command]
async fn create_session(
    state: tauri::State<'_, Arc<Desk>>,
    team_id: String,
    name: String,
    workspace: String,
    provider: Option<String>,
    model: Option<String>,
    role_description: Option<String>,
    avatar: Option<String>,
) -> Result<desk_core::Session, String> {
    state.0.lock().await.create_session_profile(
        &team_id,
        &name,
        workspace,
        provider.as_deref(),
        model.as_deref(),
        role_description.as_deref(),
        avatar.as_deref(),
    )
}

#[tauri::command]
async fn submit_message(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
    text: String,
    attachment_ids: Vec<String>,
) -> Result<String, String> {
    let ids: Vec<&str> = attachment_ids.iter().map(String::as_str).collect();
    let mut app = state.0.lock().await;
    app.submit_message(&session_id, &text, None, &ids).await
}

#[tauri::command]
async fn cancel_run(state: tauri::State<'_, Arc<Desk>>, run_id: String) -> Result<String, String> {
    state.0.lock().await.cancel_run(&run_id).await
}

#[tauri::command]
async fn stop_and_replace(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
    text: String,
    attachment_ids: Option<Vec<String>>,
) -> Result<String, String> {
    let owned = attachment_ids.unwrap_or_default();
    let ids: Vec<&str> = owned.iter().map(String::as_str).collect();
    state
        .0
        .lock()
        .await
        .stop_and_replace_with(&session_id, &text, &ids)
        .await
}

#[tauri::command]
async fn resolve_approval(
    state: tauri::State<'_, Arc<Desk>>,
    run_id: String,
    allow: bool,
) -> Result<(), String> {
    let decision = if allow {
        ApprovalDecision::AllowOnce
    } else {
        ApprovalDecision::Reject
    };
    state.0.lock().await.resolve_approval(&run_id, decision)
}

#[tauri::command]
async fn stage_attachment(
    state: tauri::State<'_, Arc<Desk>>,
    path: String,
) -> Result<desk_core::Attachment, String> {
    state.0.lock().await.stage_attachment(path)
}

#[tauri::command]
async fn stage_bytes(
    state: tauri::State<'_, Arc<Desk>>,
    name: String,
    mime: String,
    bytes: Vec<u8>,
) -> Result<desk_core::Attachment, String> {
    state.0.lock().await.stage_bytes(&name, &mime, &bytes)
}

#[tauri::command]
async fn read_image_data_url(path: String) -> Result<String, String> {
    desk_core::read_image_data_url(&path)
}

#[tauri::command]
async fn mark_read(state: tauri::State<'_, Arc<Desk>>, id: String) -> Result<(), String> {
    state.0.lock().await.mark_read(&id)
}

#[tauri::command]
async fn mark_all_read(state: tauri::State<'_, Arc<Desk>>) -> Result<(), String> {
    state.0.lock().await.mark_all_read()
}

#[tauri::command]
async fn resume_queue(state: tauri::State<'_, Arc<Desk>>) -> Result<(), String> {
    state.0.lock().await.resume_queue()
}

#[tauri::command]
async fn probe(
    state: tauri::State<'_, Arc<Desk>>,
) -> Result<Vec<desk_core::ProviderStatus>, String> {
    let ctx = state.0.lock().await.ctx();
    let (statuses, ready) = desk_core::probe_discovered(&ctx).await;
    state.0.lock().await.record_probes(statuses.clone(), ready);
    Ok(statuses)
}

#[tauri::command]
async fn install_adapter(
    state: tauri::State<'_, Arc<Desk>>,
    name: String,
) -> Result<Vec<desk_core::ProviderStatus>, String> {
    let package = desk_core::catalog::adapter_npm(&name)?.to_owned();
    desk_core::catalog::install_npm_adapter(&package).await?;
    let ctx = state.0.lock().await.ctx();
    let (statuses, ready) = desk_core::probe_discovered(&ctx).await;
    state.0.lock().await.record_probes(statuses.clone(), ready);
    Ok(statuses)
}

#[tauri::command]
async fn clock_in(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
) -> Result<desk_core::SessionView, String> {
    let ctx = state.0.lock().await.ctx();
    desk_core::clock_in_with_ctx(&ctx, &session_id).await?;
    state
        .0
        .lock()
        .await
        .snapshot()?
        .sessions
        .into_iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| "session missing after clock in".into())
}

#[tauri::command]
async fn set_session_provider(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
    provider: String,
) -> Result<desk_core::SessionView, String> {
    let mut app = state.0.lock().await;
    let _ = app.detach(&session_id).await;
    app.set_session_provider(&session_id, &provider)
}

#[tauri::command]
async fn update_session_profile(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
    name: String,
    provider: String,
    model: String,
    role_description: String,
    avatar: String,
) -> Result<desk_core::SessionView, String> {
    state
        .0
        .lock()
        .await
        .update_session_profile(
            &session_id,
            &name,
            &provider,
            &model,
            &role_description,
            &avatar,
        )
        .await
}

#[tauri::command]
async fn delete_session(
    state: tauri::State<'_, Arc<Desk>>,
    session_id: String,
) -> Result<(), String> {
    state.0.lock().await.delete_session(&session_id).await
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
            let db = data.join("desk.db");
            let desk_app = App::open(&db, default_provider()).map_err(|e| e.to_string())?;
            let desk = Arc::new(Desk(Mutex::new(desk_app)));
            app.manage(desk.clone());

            let show = MenuItem::with_id(app, "show", "打开 Desk", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .tooltip("Desk");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let _tray = tray
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                            let _ = win.emit("desk://visible", true);
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_size(tauri::LogicalSize::new(1280.0, 800.0));
                let _ = win.center();
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
                let _ = win.emit("desk://visible", true);
                #[cfg(debug_assertions)]
                {
                    let _ = win.open_devtools();
                }
            }

            let handle = app.handle().clone();
            let jump_handle = handle.clone();
            app.notification()
                .on_action(move |action| {
                    let extra = action
                        .notification()
                        .map(|n| n.extra().clone())
                        .unwrap_or_default();
                    let session_id = extra
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let notification_id = extra
                        .get("notification_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let run_id = extra
                        .get("run_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                    if let Some(win) = jump_handle.get_webview_window("main") {
                        let _ = win.show();
                        let _ = win.set_focus();
                        let _ = win.emit("desk://visible", true);
                    }
                    let _ = jump_handle.emit(
                        "desk://jump",
                        Jump {
                            session_id,
                            notification_id,
                            run_id,
                        },
                    );
                })
                .map_err(|e| e.to_string())?;

            tauri::async_runtime::spawn(async move {
                let mut seen = std::collections::HashSet::<String>::new();
                loop {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                    let snap = {
                        let mut inner = desk.0.lock().await;
                        match inner.pump().await {
                            Ok(()) => inner.snapshot(),
                            Err(_) => continue,
                        }
                    };
                    let Ok(snap) = snap else { continue };
                    for n in &snap.notifications {
                        if n.unread
                            && (n.kind == "approval" || n.kind == "error")
                            && seen.insert(n.dedupe_key.clone())
                        {
                            let _ = handle
                                .notification()
                                .builder()
                                .title(&n.title)
                                .body(&n.kind)
                                .extra("session_id", &n.session_id)
                                .extra("notification_id", &n.id)
                                .extra("run_id", n.run_id.clone().unwrap_or_default())
                                .show();
                        }
                    }
                    let _ = handle.emit("desk://snapshot", &snap);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
                let _ = window.emit("desk://visible", false);
            }
            WindowEvent::Focused(true) => {
                let _ = window.emit("desk://visible", true);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            snapshot,
            pump,
            list_messages,
            create_team,
            create_session,
            submit_message,
            cancel_run,
            stop_and_replace,
            resolve_approval,
            stage_attachment,
            stage_bytes,
            read_image_data_url,
            mark_read,
            mark_all_read,
            resume_queue,
            probe,
            install_adapter,
            clock_in,
            set_session_provider,
            update_session_profile,
            delete_session
        ])
        .run(tauri::generate_context!())
        .expect("desk failed to start");
}

fn default_provider() -> Provider {
    match std::env::var("DESK_PROVIDER").ok().as_deref() {
        Some("fake") => fake_provider(),
        Some("gemini") => Provider::gemini(
            desk_core::catalog::find_executable("DESK_GEMINI", &["gemini"])
                .unwrap_or_else(|| PathBuf::from("gemini")),
        ),
        Some("codex") => Provider::codex(
            desk_core::catalog::find_executable("DESK_CODEX_ACP", &["codex-acp"])
                .unwrap_or_else(|| PathBuf::from("codex-acp")),
        ),
        Some("claude") => Provider::claude(
            desk_core::catalog::find_executable("DESK_CLAUDE_ACP", &["claude-acp", "claude-agent-acp"])
                .unwrap_or_else(|| PathBuf::from("claude-agent-acp")),
        ),
        Some("antigravity") => Provider::antigravity(
            desk_core::catalog::find_executable("DESK_ANTIGRAVITY_ACP", &["agy_acp_server", "agy_acp_server.par"])
                .unwrap_or_else(|| PathBuf::from("agy_acp_server")),
        ),
        _ => Provider::cursor(
            desk_core::catalog::find_executable("DESK_CURSOR_AGENT", &["agent", "cursor-agent"])
                .unwrap_or_else(|| PathBuf::from("agent")),
        ),
    }
}

fn fake_provider() -> Provider {
    Provider::Fake {
        bin: fake_cli_bin(),
        scenario: std::env::var("DESK_FAKE_SCENARIO").unwrap_or_else(|_| "reply".into()),
    }
}

fn fake_cli_bin() -> PathBuf {
    if let Some(explicit) = std::env::var_os("DESK_FAKE_CLI") {
        return PathBuf::from(explicit);
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("target");
    path.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    path.push(format!("desk-fake-cli{}", std::env::consts::EXE_SUFFIX));
    path
}
