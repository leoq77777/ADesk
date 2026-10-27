use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::task::JoinSet;

use crate::catalog;
use crate::client::{
    AcpClient, ApprovalDecision, ApprovalHandle, CancelHandle, ClientEvent, EventAction,
};
use crate::scheduler::Scheduler;
use crate::store::{
    Attachment, CrashRecovery, Message, Notification, Run, Session, Store, Team,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_MESSAGE_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum Provider {
    Fake { bin: PathBuf, scenario: String },
    Acp {
        name: String,
        bin: PathBuf,
        args: Vec<String>,
        auth: Option<String>,
    },
}

impl Provider {
    pub fn name(&self) -> &str {
        match self {
            Provider::Fake { .. } => "fake",
            Provider::Acp { name, .. } => name,
        }
    }

    pub fn cursor(bin: PathBuf) -> Self {
        Self::acp("cursor", bin, &["acp"], Some("cursor_login"))
    }

    pub fn gemini(bin: PathBuf) -> Self {
        Self::acp("gemini", bin, &["--acp"], None)
    }

    pub fn codex(bin: PathBuf) -> Self {
        Self::acp("codex", bin, &[], None)
    }

    pub fn claude(bin: PathBuf) -> Self {
        Self::acp("claude", bin, &[], None)
    }

    pub fn antigravity(bin: PathBuf) -> Self {
        Self::acp("antigravity", bin, &[], None)
    }

    pub fn acp(name: &str, bin: PathBuf, args: &[&str], auth: Option<&str>) -> Self {
        Provider::Acp {
            name: name.to_owned(),
            bin,
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            auth: auth.map(str::to_owned),
        }
    }

    fn bin(&self) -> &Path {
        match self {
            Provider::Fake { bin, .. } | Provider::Acp { bin, .. } => bin,
        }
    }
}

#[derive(Debug)]
pub struct SubmitResult {
    pub run_id: String,
    pub reply: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub workspace_path: String,
    pub provider: String,
    pub cli_session_id: Option<String>,
    pub seat: String,
    pub run_id: Option<String>,
    #[serde(default)]
    pub streaming_text: Option<String>,
    #[serde(default)]
    pub pending_tool: Option<String>,
    #[serde(default)]
    pub queued_count: usize,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub role_description: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
    pub adapter_package: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelOption>,
    #[serde(default)]
    pub default_model: Option<String>,
}

impl ProviderStatus {
    fn checked(name: impl Into<String>, available: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            available,
            detail: detail.into(),
            adapter_package: None,
            models: Vec::new(),
            default_model: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub teams: Vec<Team>,
    pub sessions: Vec<SessionView>,
    pub notifications: Vec<Notification>,
    pub dispatch_paused: bool,
    pub provider: String,
    pub providers: Vec<String>,
    pub probes: Vec<ProviderStatus>,
}

#[derive(Clone)]
pub struct Ctx {
    store: Arc<Mutex<Store>>,
    providers: Arc<Mutex<HashMap<String, Provider>>>,
    default_provider: String,
    live: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<AcpClient>>>>>,
    scheduler: Arc<Mutex<Scheduler>>,
    streams: Arc<Mutex<HashMap<String, String>>>,
    pending_tools: Arc<Mutex<HashMap<String, String>>>,
}

impl Ctx {
    fn provider(&self, name: &str) -> Result<Provider, String> {
        let target = if name.is_empty() {
            self.default_provider.as_str()
        } else {
            name
        };
        if let Ok(map) = self.providers.lock() {
            if let Some(p) = map.get(target) {
                return Ok(p.clone());
            }
        }
        for found in catalog::discover_installed() {
            if let catalog::Discovery::Ready {
                name: found_name,
                bin,
                args,
                auth,
            } = found
            {
                if found_name == target {
                    let p = Provider::acp(found_name, bin, args, auth);
                    if let Ok(mut map) = self.providers.lock() {
                        map.insert(found_name.to_owned(), p.clone());
                    }
                    return Ok(p);
                }
            }
        }
        Err(format!("unknown provider {name}"))
    }
}

struct InstanceLock(File);

impl Drop for InstanceLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            let _ = libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&self.0),
                libc::LOCK_UN,
            );
        }
    }
}

pub struct App {
    ctx: Ctx,
    attachments_dir: PathBuf,
    tasks: JoinSet<(String, Result<String, String>)>,
    done: HashMap<String, Result<String, String>>,
    cancels: HashMap<String, CancelHandle>,
    approvals: HashMap<String, ApprovalHandle>,
    dispatch_paused: bool,
    probes: Vec<ProviderStatus>,
    _lock: InstanceLock,
}

impl App {
    pub fn open(db_path: impl AsRef<Path>, provider: Provider) -> Result<Self, String> {
        Self::open_with(db_path, provider, 2)
    }

    pub fn open_with(
        db_path: impl AsRef<Path>,
        provider: Provider,
        max_global: usize,
    ) -> Result<Self, String> {
        let db_path = db_path.as_ref();
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let attachments_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("attachments");
        std::fs::create_dir_all(&attachments_dir).map_err(|e| e.to_string())?;
        let lock = lock_instance(db_path)?;
        let store = Store::open(db_path)?;
        let recovery = store.recover_crash()?;
        let default_provider = provider.name().to_owned();
        let mut providers = HashMap::new();
        providers.insert(default_provider.clone(), provider);
        Ok(Self {
            ctx: Ctx {
                store: Arc::new(Mutex::new(store)),
                providers: Arc::new(Mutex::new(providers)),
                default_provider,
                live: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
                scheduler: Arc::new(Mutex::new(Scheduler::new(max_global))),
                streams: Arc::new(Mutex::new(HashMap::new())),
                pending_tools: Arc::new(Mutex::new(HashMap::new())),
            },
            attachments_dir,
            tasks: JoinSet::new(),
            done: HashMap::new(),
            cancels: HashMap::new(),
            approvals: HashMap::new(),
            dispatch_paused: recovery.interrupted > 0 || recovery.paused_queued > 0,
            probes: Vec::new(),
            _lock: lock,
        })
    }

    pub fn create_team(&self, name: &str) -> Result<Team, String> {
        let team = Team {
            id: new_id(),
            name: name.to_owned(),
        };
        self.store()?.insert_team(&team)?;
        Ok(team)
    }

    pub fn register_provider(&mut self, provider: Provider) {
        if let Ok(mut map) = self.ctx.providers.lock() {
            map.insert(provider.name().to_owned(), provider);
        }
    }

    pub async fn probe_providers(&mut self) -> Result<Vec<ProviderStatus>, String> {
        let (out, ready) = probe_discovered(&self.ctx).await;
        for provider in ready {
            self.register_provider(provider);
        }
        self.probes = out.clone();
        Ok(out)
    }

    pub fn record_probes(
        &mut self,
        statuses: Vec<ProviderStatus>,
        ready: Vec<Provider>,
    ) {
        for provider in ready {
            self.register_provider(provider);
        }
        self.probes = statuses;
    }

    pub fn create_session(
        &self,
        team_id: &str,
        name: &str,
        workspace: impl AsRef<Path>,
    ) -> Result<Session, String> {
        self.create_session_with(team_id, name, workspace, None)
    }

    pub fn create_session_with(
        &self,
        team_id: &str,
        name: &str,
        workspace: impl AsRef<Path>,
        provider: Option<&str>,
    ) -> Result<Session, String> {
        let provider_name = match provider {
            Some(name) if !name.is_empty() => {
                let known = self
                    .ctx
                    .providers
                    .lock()
                    .map(|m| m.contains_key(name))
                    .unwrap_or(false);
                if !known {
                    return Err(format!("unknown provider {name}"));
                }
                name.to_owned()
            }
            _ => self.ctx.default_provider.clone(),
        };
        let mut model_val = default_model(&provider_name);
        if let Some(status) = self.probes.iter().find(|p| p.name == provider_name) {
            if !status.available {
                return Err(format!("{provider_name} 不可用: {}", status.detail));
            }
            if let Some(def) = &status.default_model {
                model_val = def.clone();
            } else if let Some(first) = status.models.first() {
                model_val = first.id.clone();
            }
        }
        let requested = workspace.as_ref();
        let path = if requested.as_os_str().is_empty() {
            let root = self
                .attachments_dir
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("workspaces");
            let base = sanitize_dir(name);
            let mut path = root.join(&base);
            if path.exists() {
                path = root.join(format!("{base}-{}", &new_id()[..8]));
            }
            path
        } else {
            requested.to_path_buf()
        };
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let session = Session {
            id: new_id(),
            team_id: team_id.to_owned(),
            name: name.to_owned(),
            provider: provider_name.clone(),
            workspace_path: path.display().to_string(),
            cli_session_id: None,
            avatar: default_avatar(name),
            model: model_val,
            role_description: "通用研发与代码协作".to_owned(),
            created_at: now_secs(),
        };
        self.store()?.insert_session(&session)?;
        Ok(session)
    }

    pub fn create_session_profile(
        &self,
        team_id: &str,
        name: &str,
        workspace: impl AsRef<Path>,
        provider: Option<&str>,
        model: Option<&str>,
        role_description: Option<&str>,
        avatar: Option<&str>,
    ) -> Result<Session, String> {
        let provider_name = match provider {
            Some(p) if !p.trim().is_empty() => p.trim().to_owned(),
            _ => self.ctx.default_provider.clone(),
        };
        let requested_model = model.map(str::trim).filter(|s| !s.is_empty());
        let mut verified_model: Option<String> = None;
        if let Some(status) = self.probes.iter().find(|p| p.name == provider_name) {
            if !status.available {
                return Err(format!("{provider_name} 不可用: {}", status.detail));
            }
            if !status.models.is_empty() {
                if let Some(req) = requested_model {
                    let matched = status
                        .models
                        .iter()
                        .find(|m| m.id == req || m.label == req);
                    match matched {
                        Some(m) => verified_model = Some(m.id.clone()),
                        None => {
                            let valid: Vec<&str> =
                                status.models.iter().map(|m| m.id.as_str()).collect();
                            return Err(format!(
                                "{} 不支持模型 {}（可选：{}）",
                                provider_name,
                                req,
                                valid.join(", ")
                            ));
                        }
                    }
                } else if let Some(def) = &status.default_model {
                    verified_model = Some(def.clone());
                } else {
                    verified_model = Some(status.models[0].id.clone());
                }
            }
        }
        let mut session = self.create_session_with(team_id, name, workspace, provider)?;
        let avatar_val = avatar
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| session.avatar.clone());
        let model_val = verified_model
            .or_else(|| requested_model.map(str::to_owned))
            .unwrap_or_else(|| session.model.clone());
        let role_val = role_description
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| session.role_description.clone());
        self.store()?.update_session_profile(
            &session.id,
            &session.name,
            &session.provider,
            &model_val,
            &role_val,
            &avatar_val,
            false,
        )?;
        session.avatar = avatar_val;
        session.model = model_val;
        session.role_description = role_val;
        Ok(session)
    }

    pub fn set_cli_session_id(&self, session_id: &str, cli_session_id: &str) -> Result<(), String> {
        self.store()?.set_cli_session_id(session_id, cli_session_id)
    }

    pub fn get_session(&self, session_id: &str) -> Result<Session, String> {
        self.store()?.get_session(session_id)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Run, String> {
        self.store()?.get_run(run_id)
    }

    pub fn list_messages(&self, session_id: &str) -> Result<Vec<Message>, String> {
        self.store()?.list_messages(session_id)
    }

    pub fn list_messages_page(
        &self,
        session_id: &str,
        before_created_at: Option<i64>,
        before_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Message>, String> {
        self.store()?
            .list_messages_page(session_id, before_created_at, before_id, limit)
    }

    pub fn list_notifications(&self, session_id: Option<&str>) -> Result<Vec<Notification>, String> {
        self.store()?.list_notifications(session_id)
    }

    pub fn list_teams(&self) -> Result<Vec<Team>, String> {
        self.store()?.list_teams()
    }

    pub fn list_sessions(&self, team_id: Option<&str>) -> Result<Vec<Session>, String> {
        self.store()?.list_sessions(team_id)
    }

    pub fn mark_read(&self, notification_id: &str) -> Result<(), String> {
        self.store()?.mark_notification_read(notification_id)
    }

    pub fn mark_all_read(&self) -> Result<(), String> {
        self.store()?.mark_all_notifications_read()
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        let db = self.store()?;
        let teams = db.list_teams()?;
        let sessions = db.list_sessions(None)?;
        let notifications = db
            .list_notifications(None)?
            .into_iter()
            .filter(|n| n.unread && n.kind != "replied")
            .take(12)
            .collect();
        let sched = self.scheduler()?;
        let streams = self.ctx.streams.lock().ok();
        let pending_tools = self.ctx.pending_tools.lock().ok();
        let mut views = Vec::new();
        for session in sessions {
            let active_id = sched.active_run(&session.id);
            let run = match active_id {
                Some(ref id) => db.get_run(id).ok().or(db.latest_run(&session.id)?),
                None => db.latest_run(&session.id)?,
            };
            let is_active = run.as_ref().is_some_and(|r| {
                matches!(
                    r.execution_state.as_str(),
                    "starting" | "running" | "cancelling"
                )
            });
            let awaiting = is_active
                && run
                    .as_ref()
                    .and_then(|r| db.pending_approval_for_run(&r.id).ok().flatten())
                    .is_some();
            let streaming_text = run
                .as_ref()
                .filter(|_| is_active)
                .and_then(|r| streams.as_ref()?.get(&r.id).cloned())
                .filter(|s| !s.is_empty());
            let pending_tool = run
                .as_ref()
                .filter(|_| awaiting)
                .and_then(|r| pending_tools.as_ref()?.get(&r.id).cloned());
            let queued_count = sched.queued_count_for(&session.id);
            let last_error = run
                .as_ref()
                .filter(|r| matches!(r.execution_state.as_str(), "failed" | "interrupted"))
                .and_then(|r| r.error.clone());
            let avatar = if session.avatar.trim().is_empty() {
                default_avatar(&session.name)
            } else {
                session.avatar.clone()
            };
            let model = if session.model.trim().is_empty() {
                default_model(&session.provider)
            } else {
                session.model.clone()
            };
            let role_description = if session.role_description.trim().is_empty() {
                "通用研发与代码协作".to_owned()
            } else {
                session.role_description.clone()
            };
            let created_at = if session.created_at > 0 {
                session.created_at
            } else {
                run.as_ref().map(|r| r.created_at).unwrap_or(0)
            };
            views.push(SessionView {
                id: session.id,
                team_id: session.team_id,
                name: session.name,
                workspace_path: session.workspace_path,
                provider: session.provider,
                cli_session_id: session.cli_session_id,
                seat: seat_label(run.as_ref(), awaiting),
                run_id: run.map(|r| r.id),
                streaming_text,
                pending_tool,
                queued_count,
                last_error,
                avatar,
                model,
                role_description,
                created_at,
            });
        }
        let mut providers: Vec<String> = self
            .ctx
            .providers
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        providers.sort();
        Ok(Snapshot {
            teams,
            sessions: views,
            notifications,
            dispatch_paused: self.dispatch_paused,
            provider: self.ctx.default_provider.clone(),
            providers,
            probes: self.probes.clone(),
        })
    }

    pub async fn submit_message(
        &mut self,
        session_id: &str,
        text: &str,
        request_id: Option<&str>,
        attachment_ids: &[&str],
    ) -> Result<String, String> {
        let run_id = self.enqueue(session_id, text, request_id, attachment_ids)?;
        self.tick().await?;
        Ok(run_id)
    }

    pub async fn pump(&mut self) -> Result<(), String> {
        self.tick().await?;
        loop {
            match tokio::time::timeout(Duration::from_millis(0), self.tasks.join_next()).await {
                Ok(Some(finished)) => {
                    let (id, result) = finished.map_err(|e| e.to_string())?;
                    self.cancels.remove(&id);
                    self.approvals.remove(&id);
                    self.done.insert(id, result);
                }
                _ => break,
            }
        }
        Ok(())
    }

    pub fn dispatch_paused(&self) -> bool {
        self.dispatch_paused
    }

    pub fn queued_run_ids(&self) -> Result<Vec<String>, String> {
        Ok(self.scheduler()?.queued_run_ids())
    }

    pub fn active_count(&self) -> Result<usize, String> {
        Ok(self.scheduler()?.active_count())
    }

    pub fn recover_now(&self) -> Result<CrashRecovery, String> {
        self.store()?.recover_crash()
    }

    pub fn resume_queue(&mut self) -> Result<(), String> {
        self.store()?.unpause_queued()?;
        self.dispatch_paused = false;
        let queued = self.store()?.list_runs_by_state("queued")?;
        for run in queued {
            self.scheduler()?
                .enqueue(&run.session_id, &run.id, true);
        }
        Ok(())
    }

    pub fn stage_attachment(&self, path: impl AsRef<Path>) -> Result<Attachment, String> {
        let src = path.as_ref();
        let meta = std::fs::metadata(src).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("attachment is not a file".into());
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err("attachment exceeds 20 MiB".into());
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| "invalid attachment name".to_string())?
            .to_owned();
        let id = new_id();
        let dest_dir = self.attachments_dir.join(&id);
        std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
        let dest = dest_dir.join(&name);
        std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
        let att = Attachment {
            id,
            original_name: name,
            mime: guess_mime(&dest),
            size: meta.len() as i64,
            stored_path: dest.display().to_string(),
            message_id: None,
        };
        self.store()?.insert_attachment(&att)?;
        Ok(att)
    }

    pub fn stage_bytes(
        &self,
        name: &str,
        mime: &str,
        bytes: &[u8],
    ) -> Result<Attachment, String> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err("attachment exceeds 20 MiB".into());
        }
        let name = Path::new(name)
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| "invalid attachment name".to_string())?
            .to_owned();
        let id = new_id();
        let dest_dir = self.attachments_dir.join(&id);
        std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
        let dest = dest_dir.join(&name);
        std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
        let mime = if mime.trim().is_empty() {
            guess_mime(&dest)
        } else {
            mime.to_owned()
        };
        let att = Attachment {
            id,
            original_name: name,
            mime,
            size: bytes.len() as i64,
            stored_path: dest.display().to_string(),
            message_id: None,
        };
        self.store()?.insert_attachment(&att)?;
        Ok(att)
    }

    pub async fn submit(&mut self, session_id: &str, text: &str) -> Result<SubmitResult, String> {
        let run_id = self.enqueue(session_id, text, None, &[])?;
        self.wait_run(&run_id).await
    }

    pub fn enqueue(
        &self,
        session_id: &str,
        text: &str,
        request_id: Option<&str>,
        attachment_ids: &[&str],
    ) -> Result<String, String> {
        if let Some(req) = request_id {
            let existing = self.store()?.find_run_by_request(req)?;
            if let Some(existing) = existing {
                let msg = self
                    .store()?
                    .list_messages(&existing.session_id)?
                    .into_iter()
                    .find(|m| m.id == existing.message_id);
                if let Some(msg) = msg {
                    if msg.content != text {
                        return Err("request_id conflict".into());
                    }
                }
                return Ok(existing.id);
            }
        }
        let total: u64 = attachment_ids.iter().try_fold(0u64, |acc, id| {
            let att = self.store()?.get_attachment(id)?;
            Ok::<u64, String>(acc + att.size as u64)
        })?;
        if total > MAX_MESSAGE_BYTES {
            return Err("attachments exceed 50 MiB".into());
        }
        let session = self.store()?.get_session(session_id)?;
        let user = Message {
            id: new_id(),
            session_id: session.id.clone(),
            run_id: None,
            role: "user".into(),
            content: text.to_owned(),
            request_id: request_id.map(str::to_owned),
            created_at: now_secs(),
            attachments: Vec::new(),
        };
        let run = Run {
            id: new_id(),
            session_id: session.id.clone(),
            message_id: user.id.clone(),
            execution_state: "queued".into(),
            error: None,
            request_id: request_id.map(str::to_owned),
            paused: self.dispatch_paused,
            created_at: now_secs(),
        };
        self.store()?.insert_message(&user)?;
        self.store()?.insert_run(&run)?;
        for id in attachment_ids {
            self.store()?.bind_attachment(id, &user.id)?;
        }
        if !run.paused {
            self.scheduler()?
                .enqueue(&session.id, &run.id, true);
        }
        Ok(run.id)
    }

    pub async fn cancel_run(&mut self, run_id: &str) -> Result<String, String> {
        let run = self.store()?.get_run(run_id)?;
        if self.scheduler()?.cancel_queued(run_id) {
            self.store()?.set_run_state(run_id, "cancelled", None)?;
            self.store()?.expire_approvals_for_run(run_id)?;
            let _ = self.store()?.mark_run_notifications_read(run_id, "approval");
            clear_run_runtime(&self.ctx, run_id);
            notify(
                &self.ctx.store,
                &run.session_id,
                Some(run_id),
                "error",
                1,
                "已取消",
                &format!("run:{run_id}:cancelled"),
            )?;
            return Ok("cancelled".into());
        }
        if let Some(handle) = self.cancels.get(run_id) {
            self.store()?.set_run_state(run_id, "cancelling", None)?;
            self.store()?.expire_approvals_for_run(run_id)?;
            let _ = self.store()?.mark_run_notifications_read(run_id, "approval");
            handle.cancel();
            let out = self.wait_dispatched_run(run_id).await?;
            self.store()?.set_run_state(run_id, "cancelled", None)?;
            clear_run_runtime(&self.ctx, run_id);
            return Ok(out.reply);
        }
        Ok(run.execution_state)
    }

    pub async fn wait_run(&mut self, run_id: &str) -> Result<SubmitResult, String> {
        loop {
            self.tick().await?;
            if let Some(result) = self.done.remove(run_id) {
                return result.map(|reply| SubmitResult {
                    run_id: run_id.to_owned(),
                    reply,
                });
            }
            if let Some(finished) = self.tasks.join_next().await {
                let (id, result) = finished.map_err(|e| e.to_string())?;
                self.cancels.remove(&id);
                self.approvals.remove(&id);
                if id == run_id {
                    return result.map(|reply| SubmitResult {
                        run_id: id,
                        reply,
                    });
                }
                self.done.insert(id, result);
                continue;
            }
            let run = self.store()?.get_run(run_id)?;
            if run.paused {
                return Err("run paused after recovery".into());
            }
            if run.execution_state == "queued" {
                return Err("run still queued".into());
            }
            return Err(format!("run {} ended without result", run.execution_state));
        }
    }

    async fn wait_dispatched_run(&mut self, run_id: &str) -> Result<SubmitResult, String> {
        loop {
            if let Some(result) = self.done.remove(run_id) {
                return result.map(|reply| SubmitResult {
                    run_id: run_id.to_owned(),
                    reply,
                });
            }
            if let Some(finished) = self.tasks.join_next().await {
                let (id, result) = finished.map_err(|e| e.to_string())?;
                self.cancels.remove(&id);
                self.approvals.remove(&id);
                if id == run_id {
                    return result.map(|reply| SubmitResult {
                        run_id: id,
                        reply,
                    });
                }
                self.done.insert(id, result);
                continue;
            }
            let run = self.store()?.get_run(run_id)?;
            return Err(format!("run {} ended without result", run.execution_state));
        }
    }

    pub async fn tick(&mut self) -> Result<(), String> {
        if self.dispatch_paused {
            return Ok(());
        }
        let jobs = self.scheduler()?.dispatch_ready();
        for job in jobs {
            self.store()?
                .set_run_state(&job.run_id, "starting", None)?;
            let run = self.store()?.get_run(&job.run_id)?;
            let session = self.store()?.get_session(&job.session_id)?;
            let text = self
                .store()?
                .list_messages(&session.id)?
                .into_iter()
                .find(|m| m.id == run.message_id)
                .map(|m| m.content)
                .unwrap_or_default();
            let attachments = self
                .store()?
                .list_attachments_for_message(&run.message_id)?;
            let (cancel, cancel_rx) = CancelHandle::pair();
            let (approval, approval_rx) = ApprovalHandle::pair();
            self.cancels.insert(job.run_id.clone(), cancel);
            self.approvals.insert(job.run_id.clone(), approval);
            let ctx = self.ctx.clone();
            let run_id = job.run_id.clone();
            let session_id = job.session_id.clone();
            self.tasks.spawn(async move {
                let result = execute_run(
                    ctx.clone(),
                    session,
                    run_id.clone(),
                    text,
                    attachments,
                    cancel_rx,
                    approval_rx,
                )
                .await;
                if let Ok(mut sched) = ctx.scheduler.lock() {
                    sched.finish(&session_id, &run_id);
                }
                clear_run_runtime(&ctx, &run_id);
                if let Err(error) = &result {
                    persist_run_failure(&ctx.store, &session_id, &run_id, error);
                }
                (run_id, result)
            });
        }
        Ok(())
    }

    pub async fn stop_and_replace(
        &mut self,
        session_id: &str,
        text: &str,
    ) -> Result<String, String> {
        self.stop_and_replace_with(session_id, text, &[]).await
    }

    pub async fn stop_and_replace_with(
        &mut self,
        session_id: &str,
        text: &str,
        attachment_ids: &[&str],
    ) -> Result<String, String> {
        let active = self.scheduler()?.active_run(session_id);
        if let Some(run_id) = active {
            let _ = self.cancel_run(&run_id).await?;
        }
        let run_id = self.enqueue(session_id, text, None, attachment_ids)?;
        if let Ok(mut sched) = self.ctx.scheduler.lock() {
            sched.cancel_queued(&run_id);
            sched.enqueue_front(session_id, &run_id);
        }
        self.tick().await?;
        Ok(run_id)
    }

    pub fn resolve_approval(
        &self,
        run_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let approval_id = self
            .store()?
            .pending_approval_for_run(run_id)?
            .ok_or_else(|| "no pending approval".to_string())?;
        let handle = self
            .approvals
            .get(run_id)
            .ok_or_else(|| "approval handle missing".to_string())?;
        handle.resolve(decision);
        let status = match decision {
            ApprovalDecision::AllowOnce => "allowed",
            ApprovalDecision::Reject => "rejected",
        };
        if let Ok(mut tools) = self.ctx.pending_tools.lock() {
            tools.remove(run_id);
        }
        let _ = self.store()?.mark_run_notifications_read(run_id, "approval");
        self.store()?.set_approval_status(&approval_id, status)
    }

    pub async fn detach(&mut self, session_id: &str) -> Result<(), String> {
        evict_live(&self.ctx, session_id).await;
        Ok(())
    }

    pub fn set_session_provider(
        &self,
        session_id: &str,
        provider: &str,
    ) -> Result<SessionView, String> {
        let provider_name = provider.trim();
        if provider_name.is_empty() {
            return Err("provider 不能为空".into());
        }
        let _ = self.ctx.provider(provider_name)?;
        self.store()?.set_session_provider(session_id, provider_name)?;
        self.snapshot()?
            .sessions
            .into_iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| "session missing".into())
    }

    pub async fn update_session_profile(
        &mut self,
        session_id: &str,
        name: &str,
        provider: &str,
        _model: &str,
        role_description: &str,
        avatar: &str,
    ) -> Result<SessionView, String> {
        let current = self.store()?.get_session(session_id)?;
        let name = if name.trim().is_empty() {
            current.name.as_str()
        } else {
            name.trim()
        };
        let provider = if provider.trim().is_empty() {
            current.provider.as_str()
        } else {
            provider.trim()
        };
        let _ = self.ctx.provider(provider)?;
        let model = current.model.clone();
        let role_description = if role_description.trim().is_empty() {
            "通用研发与代码协作"
        } else {
            role_description.trim()
        };
        let avatar = if avatar.trim().is_empty() {
            default_avatar(name)
        } else {
            avatar.trim().to_owned()
        };
        let reset_cli = current.provider != provider;
        if reset_cli {
            evict_live(&self.ctx, session_id).await;
        }
        self.store()?.update_session_profile(
            session_id,
            name,
            provider,
            &model,
            role_description,
            &avatar,
            reset_cli,
        )?;
        self.snapshot()?
            .sessions
            .into_iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| "session missing".into())
    }

    pub async fn delete_session(&mut self, session_id: &str) -> Result<(), String> {
        let active_run = self.scheduler()?.clear_session(session_id);
        if let Some(run_id) = active_run {
            if let Some(cancel) = self.cancels.remove(&run_id) {
                cancel.cancel();
            }
            self.approvals.remove(&run_id);
            clear_run_runtime(&self.ctx, &run_id);
        }
        evict_live(&self.ctx, session_id).await;
        self.store()?.delete_session(session_id)?;
        Ok(())
    }

    pub fn ctx(&self) -> Ctx {
        self.ctx.clone()
    }

    pub async fn clock_in(&mut self, session_id: &str) -> Result<SessionView, String> {
        clock_in_with_ctx(&self.ctx, session_id).await?;
        self.snapshot()?
            .sessions
            .into_iter()
            .find(|s| s.id == session_id)
            .ok_or_else(|| "session missing after clock in".into())
    }

    fn store(&self) -> Result<std::sync::MutexGuard<'_, Store>, String> {
        self.ctx.store.lock().map_err(|e| e.to_string())
    }

    fn scheduler(&self) -> Result<std::sync::MutexGuard<'_, Scheduler>, String> {
        self.ctx.scheduler.lock().map_err(|e| e.to_string())
    }
}

pub async fn clock_in_with_ctx(ctx: &Ctx, session_id: &str) -> Result<(), String> {
    evict_live(ctx, session_id).await;
    lock_store(ctx)?.clear_cli_session_id(session_id)?;
    let session = lock_store(ctx)?.get_session(session_id)?;
    if let Err(err) = ensure_client(ctx, &session).await {
        if is_unusable_provider_error(&err) && session.provider != ctx.default_provider {
            let fallback = ctx.default_provider.clone();
            lock_store(ctx)?.set_session_provider(session_id, &fallback)?;
            let updated = lock_store(ctx)?.get_session(session_id)?;
            if let Err(err2) = ensure_client(ctx, &updated).await {
                let _ = lock_store(ctx)?.insert_message(&Message {
                    id: new_id(),
                    session_id: session_id.to_owned(),
                    run_id: None,
                    role: "error".into(),
                    content: format!("恢复上班失败：{err2}"),
                    request_id: None,
                    created_at: now_secs(),
                    attachments: Vec::new(),
                });
                return Err(err2);
            }
        } else {
            let _ = lock_store(ctx)?.insert_message(&Message {
                id: new_id(),
                session_id: session_id.to_owned(),
                run_id: None,
                role: "error".into(),
                content: format!("恢复上班失败：{err}"),
                request_id: None,
                created_at: now_secs(),
                attachments: Vec::new(),
            });
            return Err(err);
        }
    }
    if let Some(run) = lock_store(ctx)?.latest_run(session_id)? {
        if matches!(run.execution_state.as_str(), "failed" | "interrupted") {
            lock_store(ctx)?.set_run_state(&run.id, "cancelled", None)?;
            lock_store(ctx)?.expire_approvals_for_run(&run.id)?;
        }
    }
    let _ = lock_store(ctx)?.mark_session_notifications_read(session_id);
    lock_store(ctx)?.insert_message(&Message {
        id: new_id(),
        session_id: session_id.to_owned(),
        run_id: None,
        role: "assistant".into(),
        content: "已重新到岗。".into(),
        request_id: None,
        created_at: now_secs(),
        attachments: Vec::new(),
    })?;
    notify(
        &ctx.store,
        session_id,
        None,
        "info",
        2,
        "已到岗",
        &format!("session:{session_id}:clock-in:{}", now_secs()),
    )?;
    Ok(())
}

async fn execute_run(
    ctx: Ctx,
    session: Session,
    run_id: String,
    text: String,
    attachments: Vec<Attachment>,
    cancel: tokio::sync::watch::Receiver<bool>,
    approval: tokio::sync::watch::Receiver<Option<ApprovalDecision>>,
) -> Result<String, String> {
    let client = ensure_client(&ctx, &session).await?;
    let cli_id = lock_store(&ctx)?
        .get_session(&session.id)?
        .cli_session_id
        .ok_or_else(|| "missing cli session id".to_string())?;
    lock_store(&ctx)?.set_run_state(&run_id, "running", None)?;
    let provider = ctx.provider(&session.provider)?;
    let parts = crate::prompt::encode_prompt(&provider, &text, &attachments)?;
    let mut client = client.lock().await;
    let make_on_event = || {
        let ctx_cb = ctx.clone();
        let session_id = session.id.clone();
        let run_id_cb = run_id.clone();
        move |ev: &ClientEvent| on_run_event(&ctx_cb, &session_id, &run_id_cb, ev)
    };
    let out = match client
        .prompt_parts(
            &cli_id,
            parts.clone(),
            cancel.clone(),
            approval.clone(),
            make_on_event(),
        )
        .await
    {
        Ok(out) => out,
        Err(err) => {
            let explained = client.explain(&err);
            if !is_missing_cli_session(&explained) {
                return Err(explained);
            }
            drop(client);
            evict_live(&ctx, &session.id).await;
            lock_store(&ctx)?.clear_cli_session_id(&session.id)?;
            clear_run_runtime(&ctx, &run_id);
            let client = ensure_client(&ctx, &session).await?;
            let cli_id = lock_store(&ctx)?
                .get_session(&session.id)?
                .cli_session_id
                .ok_or_else(|| "missing cli session id".to_string())?;
            let mut client = client.lock().await;
            client
                .prompt_parts(&cli_id, parts, cancel, approval, make_on_event())
                .await
                .map_err(|e| client.explain(&e))?
        }
    };

    clear_run_runtime(&ctx, &run_id);
    let _ = lock_store(&ctx)?.expire_approvals_for_run(&run_id);
    let _ = lock_store(&ctx)?.mark_run_notifications_read(&run_id, "approval");

    if out.stop_reason == "cancelled" {
        lock_store(&ctx)?.set_run_state(&run_id, "cancelled", None)?;
        notify(
            &ctx.store,
            &session.id,
            Some(&run_id),
            "error",
            1,
            "已取消",
            &format!("run:{run_id}:cancelled"),
        )?;
        return Ok(out.text);
    }

    lock_store(&ctx)?.insert_message(&Message {
        id: new_id(),
        session_id: session.id.clone(),
        run_id: Some(run_id.clone()),
        role: "assistant".into(),
        content: out.text.clone(),
        request_id: None,
        created_at: now_secs(),
        attachments: Vec::new(),
    })?;
    lock_store(&ctx)?.set_run_state(&run_id, "succeeded", None)?;
    notify(
        &ctx.store,
        &session.id,
        Some(&run_id),
        "replied",
        3,
        "本轮已结束",
        &format!("run:{run_id}:replied"),
    )?;
    Ok(out.text)
}

async fn ensure_client(
    ctx: &Ctx,
    session: &Session,
) -> Result<Arc<tokio::sync::Mutex<AcpClient>>, String> {
    let mut live = ctx.live.lock().await;
    if let Some(existing) = live.get(&session.id) {
        return Ok(existing.clone());
    }
    let cwd = Path::new(&session.workspace_path);
    let provider = ctx.provider(&session.provider)?;
    let mut client = spawn_provider(&provider, cwd).await?;
    client.initialize().await.map_err(|e| client.explain(&e))?;
    if let Provider::Acp {
        auth: Some(ref method),
        ..
    } = provider
    {
        client
            .authenticate(method)
            .await
            .map_err(|e| client.explain(&e))?;
    }
    let latest = lock_store(ctx)?.get_session(&session.id)?;
    let cwd = cwd
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(&session.workspace_path));
    let cwd_str = cwd.to_string_lossy();
    let active_cli_id = if let Some(cli_id) = &latest.cli_session_id {
        if let Err(err) = client.session_load(cli_id, cwd_str.as_ref()).await {
            let explained = client.explain(&err);
            if !is_missing_cli_session(&explained) {
                return Err(explained);
            }
            let new_id = client
                .session_new(cwd_str.as_ref())
                .await
                .map_err(|e| client.explain(&e))?;
            lock_store(ctx)?.set_cli_session_id(&session.id, &new_id)?;
            new_id
        } else {
            cli_id.clone()
        }
    } else {
        let new_id = client
            .session_new(cwd_str.as_ref())
            .await
            .map_err(|e| client.explain(&e))?;
        lock_store(ctx)?.set_cli_session_id(&session.id, &new_id)?;
        new_id
    };
    if !latest.model.trim().is_empty() {
        let _ = client
            .set_session_model(&active_cli_id, &latest.model)
            .await;
    }
    let wrapped = Arc::new(tokio::sync::Mutex::new(client));
    live.insert(session.id.clone(), wrapped.clone());
    Ok(wrapped)
}

pub async fn probe_discovered(ctx: &Ctx) -> (Vec<ProviderStatus>, Vec<Provider>) {
    enum Slot {
        Fixed(ProviderStatus),
        Pending(usize, Provider),
    }
    let mut slots = Vec::new();
    let mut set = JoinSet::new();
    let mut seen = HashSet::new();

    let fake_opt = ctx
        .providers
        .lock()
        .ok()
        .and_then(|m| m.get("fake").cloned());
    if let Some(fake) = fake_opt {
        let idx = slots.len();
        slots.push(Slot::Pending(idx, fake.clone()));
        set.spawn(async move { (idx, probe_one(&fake).await) });
        seen.insert("fake".to_owned());
    }
    for found in catalog::discover_installed() {
        match found {
            catalog::Discovery::NeedsAdapter {
                name,
                detail,
                package,
            } => {
                if seen.insert(name.to_owned()) {
                    slots.push(Slot::Fixed(ProviderStatus {
                        name: name.to_owned(),
                        available: false,
                        detail,
                        adapter_package: package.map(str::to_owned),
                        models: Vec::new(),
                        default_model: None,
                    }));
                }
            }
            catalog::Discovery::Ready {
                name,
                bin,
                args,
                auth,
            } => {
                if !seen.insert(name.to_owned()) {
                    continue;
                }
                let provider = Provider::acp(name, bin, args, auth);
                let idx = slots.len();
                let p_clone = provider.clone();
                slots.push(Slot::Pending(idx, provider));
                set.spawn(async move { (idx, probe_one(&p_clone).await) });
            }
        }
    }
    let mut results = HashMap::new();
    while let Some(res) = set.join_next().await {
        if let Ok((idx, status)) = res {
            results.insert(idx, status);
        }
    }
    let mut out = Vec::with_capacity(slots.len());
    let mut ready = Vec::new();
    for slot in slots {
        match slot {
            Slot::Fixed(status) => out.push(status),
            Slot::Pending(idx, provider) => {
                if let Some(status) = results.remove(&idx) {
                    if status.available {
                        ready.push(provider);
                    }
                    out.push(status);
                }
            }
        }
    }
    (out, ready)
}

async fn probe_one(provider: &Provider) -> ProviderStatus {
    let name = provider.name().to_owned();
    let bin = provider.bin().to_path_buf();
    if !bin.is_file() {
        return ProviderStatus::checked(name, false, format!("未找到 {}", bin.display()));
    }
    let cwd = std::env::temp_dir().join(format!("desk-probe-{name}"));
    let _ = std::fs::create_dir_all(&cwd);
    let ran = tokio::time::timeout(Duration::from_secs(9), async {
        let mut client = spawn_provider(provider, &cwd).await?;
        let init = client.initialize().await.map_err(|e| client.explain(&e))?;
        if let Provider::Acp {
            auth: Some(method), ..
        } = provider
        {
            client
                .authenticate(method)
                .await
                .map_err(|e| client.explain(&e))?;
        }
        let sess = client
            .session_new_raw(&cwd.to_string_lossy())
            .await
            .map_err(|e| client.explain(&e))?;
        if sess.get("sessionId").and_then(Value::as_str).is_none() {
            return Err("session/new missing sessionId".into());
        }
        let _ = client.shutdown().await;
        Ok::<(Value, Value), String>((init, sess))
    })
    .await;
    match ran {
        Ok(Ok((init, sess))) => {
            let info = init.get("agentInfo").or_else(|| init.get("serverInfo"));
            let detail = info
                .and_then(|s| {
                    let n = s.get("name").and_then(Value::as_str).unwrap_or("");
                    let v = s.get("version").and_then(Value::as_str).unwrap_or("");
                    let text = format!("{n} {v}").trim().to_owned();
                    (!text.is_empty()).then_some(text)
                })
                .or_else(|| {
                    init.get("protocolVersion")
                        .map(|v| format!("protocol {v}"))
                })
                .unwrap_or_else(|| bin.display().to_string());
            let (models, default_model) = extract_session_models(&sess);
            ProviderStatus {
                name,
                available: true,
                detail,
                adapter_package: None,
                models,
                default_model,
            }
        }
        Ok(Err(err)) => ProviderStatus::checked(name, false, err),
        Err(_) => ProviderStatus::checked(name, false, "ACP 验证超时"),
    }
}

fn extract_session_models(sess: &Value) -> (Vec<ModelOption>, Option<String>) {
    let mut models = Vec::new();
    let mut default_model = None;

    if let Some(opts) = sess.get("configOptions").and_then(Value::as_array) {
        if let Some(model_cfg) = opts.iter().find(|o| {
            o.get("id").and_then(Value::as_str) == Some("model")
                || o.get("category").and_then(Value::as_str) == Some("model")
        }) {
            default_model = model_cfg
                .get("currentValue")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(items) = model_cfg.get("options").and_then(Value::as_array) {
                for item in items {
                    let id = item
                        .get("value")
                        .or_else(|| item.get("modelId"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if id.is_empty() {
                        continue;
                    }
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .trim();
                    let label = if name.is_empty() || name == id {
                        id.to_owned()
                    } else {
                        format!("{name} ({id})")
                    };
                    models.push(ModelOption {
                        id: id.to_owned(),
                        label,
                    });
                }
            }
        }
    }

    if models.is_empty() {
        if let Some(models_obj) = sess.get("models") {
            if default_model.is_none() {
                default_model = models_obj
                    .get("currentModelId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            if let Some(avail) = models_obj.get("availableModels").and_then(Value::as_array) {
                for item in avail {
                    let id = item
                        .get("modelId")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if id.is_empty() {
                        continue;
                    }
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .trim();
                    let label = if name.is_empty() || name == id {
                        id.to_owned()
                    } else {
                        format!("{name} ({id})")
                    };
                    models.push(ModelOption {
                        id: id.to_owned(),
                        label,
                    });
                }
            }
        }
    }

    if default_model.is_none() {
        default_model = models.first().map(|m| m.id.clone());
    }
    (models, default_model)
}

fn persist_run_failure(
    store: &Arc<Mutex<Store>>,
    session_id: &str,
    run_id: &str,
    error: &str,
) {
    let away = is_missing_cli_session(error);
    let content = if away {
        "员工已下班，CLI 会话已失效。点「恢复上班」后再发消息。".to_owned()
    } else {
        error.to_owned()
    };
    if let Ok(db) = store.lock() {
        let _ = db.set_run_state(run_id, "failed", Some(error));
        let _ = db.expire_approvals_for_run(run_id);
        let _ = db.mark_run_notifications_read(run_id, "approval");
        let _ = db.insert_message(&Message {
            id: new_id(),
            session_id: session_id.to_owned(),
            run_id: Some(run_id.to_owned()),
            role: "error".into(),
            content,
            request_id: None,
            created_at: now_secs(),
            attachments: Vec::new(),
        });
    }
    let title = if away {
        "已下班".to_owned()
    } else {
        let title = error.lines().next().unwrap_or("执行失败");
        if title.chars().count() > 48 {
            format!("{}…", title.chars().take(48).collect::<String>())
        } else {
            title.to_owned()
        }
    };
    let _ = notify(
        store,
        session_id,
        Some(run_id),
        if away { "away" } else { "error" },
        1,
        &title,
        &format!("run:{run_id}:failed"),
    );
}

async fn evict_live(ctx: &Ctx, session_id: &str) {
    let client = {
        let mut live = ctx.live.lock().await;
        live.remove(session_id)
    };
    if let Some(client) = client {
        let mut client = client.lock().await;
        let _ = client.shutdown().await;
    }
}

async fn spawn_provider(provider: &Provider, cwd: &Path) -> Result<AcpClient, String> {
    match provider {
        Provider::Fake { bin, scenario } => {
            AcpClient::spawn_args(bin, cwd, &["--scenario", scenario])
                .await
                .map_err(|e| e.to_string())
        }
        Provider::Acp { bin, args, .. } => {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            AcpClient::spawn_args(bin, cwd, &args)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

fn lock_instance(db_path: &Path) -> Result<InstanceLock, String> {
    let path = db_path.with_extension("lock");
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        let rc = unsafe { libc::flock(std::os::unix::io::AsRawFd::as_raw_fd(&file), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err("another Desk instance has this database".into());
        }
    }
    Ok(InstanceLock(file))
}

fn lock_store(ctx: &Ctx) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    ctx.store.lock().map_err(|e| e.to_string())
}

fn lock_store_ref(store: &Arc<Mutex<Store>>) -> Result<std::sync::MutexGuard<'_, Store>, String> {
    store.lock().map_err(|e| e.to_string())
}

fn notify(
    store: &Arc<Mutex<Store>>,
    session_id: &str,
    run_id: Option<&str>,
    kind: &str,
    priority: i64,
    title: &str,
    dedupe_key: &str,
) -> Result<(), String> {
    lock_store_ref(store)?.insert_notification(&Notification {
        id: new_id(),
        session_id: session_id.to_owned(),
        run_id: run_id.map(str::to_owned),
        kind: kind.to_owned(),
        priority,
        title: title.to_owned(),
        unread: true,
        dedupe_key: dedupe_key.to_owned(),
        created_at: now_secs(),
    })?;
    Ok(())
}

pub fn read_image_data_url(path: &str) -> Result<String, String> {
    let raw = path.trim();
    let stripped = raw.strip_prefix("file://").unwrap_or(raw);
    let p = Path::new(stripped);
    let meta = std::fs::metadata(p).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("image unavailable or exceeds 20 MiB".into());
    }
    let mime = match p
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        _ => "image/png",
    };
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

fn guess_mime(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("txt") | Some("md") => "text/plain",
        _ => "application/octet-stream",
    }
    .to_owned()
}

fn clear_run_runtime(ctx: &Ctx, run_id: &str) {
    if let Ok(mut streams) = ctx.streams.lock() {
        streams.remove(run_id);
    }
    if let Ok(mut tools) = ctx.pending_tools.lock() {
        tools.remove(run_id);
    }
}

fn on_run_event(
    ctx: &Ctx,
    session_id: &str,
    run_id: &str,
    ev: &ClientEvent,
) -> EventAction {
    match ev {
        ClientEvent::AssistantDelta(delta) => {
            if let Ok(mut streams) = ctx.streams.lock() {
                let buf = streams.entry(run_id.to_owned()).or_default();
                buf.push_str(delta);
            }
            EventAction::Continue
        }
        ClientEvent::ApprovalRequested { request_id, tool } => {
            let req = request_id.to_string();
            if let Ok(mut tools) = ctx.pending_tools.lock() {
                tools.insert(run_id.to_owned(), tool.clone());
            }
            if let Ok(db) = ctx.store.lock() {
                let _ = db.insert_approval(&new_id(), run_id, &req, "pending");
            }
            let _ = notify(
                &ctx.store,
                session_id,
                Some(run_id),
                "approval",
                0,
                &format!("待审批: {tool}"),
                &format!("run:{run_id}:approval:{req}"),
            );
            EventAction::Wait
        }
        _ => EventAction::Continue,
    }
}

fn is_missing_cli_session(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    (e.contains("session") && e.contains("not found"))
        || e.contains("unknown session")
        || e.contains("no such session")
}

fn is_unusable_provider_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("no such file or directory")
        || e.contains("unknown provider")
        || e.contains("未找到")
        || e.contains("no longer supported")
        || e.contains("antigravity.google")
        || e.contains("握手响应超时")
}

fn seat_label(run: Option<&Run>, awaiting_approval: bool) -> String {
    if awaiting_approval {
        return "awaiting_approval".into();
    }
    match run.map(|r| r.execution_state.as_str()) {
        Some("queued") => "queued".into(),
        Some("starting" | "running" | "cancelling") => "working".into(),
        Some("failed" | "interrupted") => {
            if run
                .and_then(|r| r.error.as_deref())
                .is_some_and(is_missing_cli_session)
            {
                "away".into()
            } else {
                "error".into()
            }
        }
        _ => "idle".into(),
    }
}

fn sanitize_dir(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || ('\u{4e00}'..='\u{9fff}').contains(&c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_');
    if s.is_empty() {
        "seat".into()
    } else {
        s.to_owned()
    }
}

fn default_avatar(name: &str) -> String {
    const CAST: [&str; 8] = [
        "sebastian",
        "maru",
        "abigail",
        "harvey",
        "elliott",
        "penny",
        "clint",
        "robin",
    ];
    let lower = name.to_ascii_lowercase();
    for c in CAST {
        if lower.contains(c) {
            return c.to_owned();
        }
    }
    let mut hash = 0usize;
    for b in name.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(b as usize);
    }
    CAST[hash % CAST.len()].to_owned()
}

fn default_model(provider: &str) -> String {
    match provider {
        "claude" => "default".into(),
        "codex" => "gpt-6.1-sol".into(),
        "gemini" | "antigravity" => "gemini-3.8-flash".into(),
        "cursor" => "default[]".into(),
        "fake" => "fake-default".into(),
        _ => "default".into(),
    }
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

static LAST_TS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn now_secs() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let mut prev = LAST_TS.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        let next = now.max(prev + 1);
        match LAST_TS.compare_exchange_weak(
            prev,
            next,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => prev = actual,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(error: &str) -> Run {
        Run {
            id: "r1".into(),
            session_id: "s1".into(),
            message_id: "m1".into(),
            execution_state: "failed".into(),
            error: Some(error.into()),
            request_id: None,
            paused: false,
            created_at: 1,
        }
    }

    #[test]
    fn missing_cli_session_seat_is_away() {
        let run = failed(r#"Invalid params: {"message":"Session \"01a16166\" not found"}"#);
        assert_eq!(seat_label(Some(&run), false), "away");
        assert_eq!(seat_label(Some(&failed("quota exceeded")), false), "error");
    }
}
