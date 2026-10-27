use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub provider: String,
    pub workspace_path: String,
    pub cli_session_id: Option<String>,
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub role_description: String,
    #[serde(default)]
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content: String,
    pub request_id: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub session_id: String,
    pub message_id: String,
    pub execution_state: String,
    pub error: Option<String>,
    pub request_id: Option<String>,
    pub paused: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub original_name: String,
    pub mime: String,
    pub size: i64,
    pub stored_path: String,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub kind: String,
    pub priority: i64,
    pub title: String,
    pub unread: bool,
    pub dedupe_key: String,
    pub created_at: i64,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL,
                name TEXT NOT NULL,
                provider TEXT NOT NULL,
                workspace_path TEXT NOT NULL,
                cli_session_id TEXT,
                avatar TEXT NOT NULL DEFAULT '',
                model TEXT NOT NULL DEFAULT '',
                role_description TEXT NOT NULL DEFAULT '',
                created_at INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(team_id) REFERENCES teams(id)
             );",
        )
        .map_err(err)?;
        // Migrate existing databases that lack the new profile columns
        for col_sql in [
            "ALTER TABLE sessions ADD COLUMN avatar TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE sessions ADD COLUMN model TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE sessions ADD COLUMN role_description TEXT NOT NULL DEFAULT ''",
            "ALTER TABLE sessions ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0",
        ] {
            let _ = conn.execute(col_sql, []);
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                run_id TEXT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                request_id TEXT,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
             );
             CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                execution_state TEXT NOT NULL,
                error TEXT,
                request_id TEXT,
                paused INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
             );
             CREATE UNIQUE INDEX IF NOT EXISTS runs_request_id
                ON runs(request_id) WHERE request_id IS NOT NULL;
             CREATE TABLE IF NOT EXISTS attachments (
                id TEXT PRIMARY KEY,
                original_name TEXT NOT NULL,
                mime TEXT NOT NULL,
                size INTEGER NOT NULL,
                stored_path TEXT NOT NULL,
                message_id TEXT
             );
             CREATE TABLE IF NOT EXISTS notifications (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                run_id TEXT,
                kind TEXT NOT NULL,
                priority INTEGER NOT NULL,
                title TEXT NOT NULL,
                unread INTEGER NOT NULL,
                dedupe_key TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS approvals (
                id TEXT PRIMARY KEY,
                run_id TEXT NOT NULL,
                provider_request_id TEXT NOT NULL,
                status TEXT NOT NULL
             );",
        )
        .map_err(err)?;
        Ok(Self { conn })
    }

    pub fn insert_team(&self, team: &Team) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO teams (id, name) VALUES (?1, ?2)",
                params![team.id, team.name],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn insert_session(&self, session: &Session) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO sessions (id, team_id, name, provider, workspace_path, cli_session_id, avatar, model, role_description, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    session.id,
                    session.team_id,
                    session.name,
                    session.provider,
                    session.workspace_path,
                    session.cli_session_id,
                    session.avatar,
                    session.model,
                    session.role_description,
                    session.created_at
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn list_teams(&self) -> Result<Vec<Team>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name FROM teams ORDER BY name, id")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Team {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)
    }

    pub fn list_sessions(&self, team_id: Option<&str>) -> Result<Vec<Session>, String> {
        if let Some(team_id) = team_id {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, team_id, name, provider, workspace_path, cli_session_id, avatar, model, role_description, created_at
                     FROM sessions WHERE team_id = ?1 ORDER BY name, id",
                )
                .map_err(err)?;
            let rows = stmt
                .query_map([team_id], row_to_session)
                .map_err(err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(err)
        } else {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, team_id, name, provider, workspace_path, cli_session_id, avatar, model, role_description, created_at
                     FROM sessions ORDER BY name, id",
                )
                .map_err(err)?;
            let rows = stmt.query_map([], row_to_session).map_err(err)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(err)
        }
    }

    pub fn latest_run(&self, session_id: &str) -> Result<Option<Run>, String> {
        self.conn
            .query_row(
                "SELECT id, session_id, message_id, execution_state, error, request_id, paused, created_at
                 FROM runs WHERE session_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
                [session_id],
                row_to_run,
            )
            .optional()
            .map_err(err)
    }

    pub fn mark_notification_read(&self, id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE notifications SET unread = 0 WHERE id = ?1",
                [id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn mark_run_notifications_read(&self, run_id: &str, kind: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE notifications SET unread = 0 WHERE run_id = ?1 AND kind = ?2",
                params![run_id, kind],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn mark_session_notifications_read(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE notifications SET unread = 0 WHERE session_id = ?1",
                [session_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn mark_all_notifications_read(&self) -> Result<(), String> {
        self.conn
            .execute("UPDATE notifications SET unread = 0 WHERE unread != 0", [])
            .map_err(err)?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Session, String> {
        self.conn
            .query_row(
                "SELECT id, team_id, name, provider, workspace_path, cli_session_id, avatar, model, role_description, created_at
                 FROM sessions WHERE id = ?1",
                [id],
                row_to_session,
            )
            .map_err(err)
    }

    pub fn set_cli_session_id(&self, session_id: &str, cli_session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET cli_session_id = ?1 WHERE id = ?2",
                params![cli_session_id, session_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn clear_cli_session_id(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET cli_session_id = NULL WHERE id = ?1",
                [session_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn set_session_provider(&self, session_id: &str, provider: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE sessions SET provider = ?1, cli_session_id = NULL WHERE id = ?2",
                params![provider, session_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn update_session_profile(
        &self,
        session_id: &str,
        name: &str,
        provider: &str,
        model: &str,
        role_description: &str,
        avatar: &str,
        reset_cli_session: bool,
    ) -> Result<(), String> {
        if reset_cli_session {
            self.conn
                .execute(
                    "UPDATE sessions
                     SET name = ?1, provider = ?2, model = ?3, role_description = ?4, avatar = ?5, cli_session_id = NULL
                     WHERE id = ?6",
                    params![name, provider, model, role_description, avatar, session_id],
                )
                .map_err(err)?;
        } else {
            self.conn
                .execute(
                    "UPDATE sessions
                     SET name = ?1, provider = ?2, model = ?3, role_description = ?4, avatar = ?5
                     WHERE id = ?6",
                    params![name, provider, model, role_description, avatar, session_id],
                )
                .map_err(err)?;
        }
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM approvals WHERE run_id IN (SELECT id FROM runs WHERE session_id = ?1)",
                [session_id],
            )
            .map_err(err)?;
        self.conn
            .execute(
                "DELETE FROM attachments WHERE message_id IN (SELECT id FROM messages WHERE session_id = ?1)",
                [session_id],
            )
            .map_err(err)?;
        self.conn
            .execute("DELETE FROM notifications WHERE session_id = ?1", [session_id])
            .map_err(err)?;
        self.conn
            .execute("DELETE FROM runs WHERE session_id = ?1", [session_id])
            .map_err(err)?;
        self.conn
            .execute("DELETE FROM messages WHERE session_id = ?1", [session_id])
            .map_err(err)?;
        self.conn
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
            .map_err(err)?;
        Ok(())
    }

    pub fn insert_message(&self, message: &Message) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO messages (id, session_id, run_id, role, content, request_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    message.id,
                    message.session_id,
                    message.run_id,
                    message.role,
                    message.content,
                    message.request_id,
                    message.created_at
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn list_messages(&self, session_id: &str) -> Result<Vec<Message>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, run_id, role, content, request_id, created_at
                 FROM messages WHERE session_id = ?1 ORDER BY created_at, id",
            )
            .map_err(err)?;
        let rows = stmt.query_map([session_id], row_to_message).map_err(err)?;
        let mut out = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
        for msg in &mut out {
            msg.attachments = self.list_attachments_for_message(&msg.id).unwrap_or_default();
        }
        Ok(out)
    }

    pub fn list_messages_page(
        &self,
        session_id: &str,
        before_created_at: Option<i64>,
        before_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Message>, String> {
        let limit = limit.clamp(1, 500) as i64;
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, run_id, role, content, request_id, created_at
                 FROM messages
                 WHERE session_id = ?1
                   AND (
                     ?2 IS NULL
                     OR created_at < ?2
                     OR (created_at = ?2 AND id < ?3)
                   )
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?4",
            )
            .map_err(err)?;
        let before_id = before_id.unwrap_or("");
        let rows = stmt
            .query_map(
                params![session_id, before_created_at, before_id, limit],
                row_to_message,
            )
            .map_err(err)?;
        let mut out = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
        out.reverse();
        for msg in &mut out {
            msg.attachments = self.list_attachments_for_message(&msg.id).unwrap_or_default();
        }
        Ok(out)
    }

    pub fn insert_run(&self, run: &Run) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO runs (id, session_id, message_id, execution_state, error, request_id, paused, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    run.id,
                    run.session_id,
                    run.message_id,
                    run.execution_state,
                    run.error,
                    run.request_id,
                    run.paused as i64,
                    run.created_at
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Run, String> {
        self.conn
            .query_row(
                "SELECT id, session_id, message_id, execution_state, error, request_id, paused, created_at
                 FROM runs WHERE id = ?1",
                [id],
                row_to_run,
            )
            .map_err(err)
    }

    pub fn find_run_by_request(&self, request_id: &str) -> Result<Option<Run>, String> {
        self.conn
            .query_row(
                "SELECT id, session_id, message_id, execution_state, error, request_id, paused, created_at
                 FROM runs WHERE request_id = ?1",
                [request_id],
                row_to_run,
            )
            .optional()
            .map_err(err)
    }

    pub fn set_run_state(
        &self,
        run_id: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE runs SET execution_state = ?1, error = ?2 WHERE id = ?3",
                params![state, error, run_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn set_run_paused(&self, run_id: &str, paused: bool) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE runs SET paused = ?1 WHERE id = ?2",
                params![paused as i64, run_id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn recover_crash(&self) -> Result<CrashRecovery, String> {
        let interrupted = self
            .conn
            .execute(
                "UPDATE runs SET execution_state = 'interrupted'
                 WHERE execution_state IN ('starting', 'running', 'cancelling')",
                [],
            )
            .map_err(err)?;
        let expired = self
            .conn
            .execute(
                "UPDATE approvals SET status = 'expired' WHERE status = 'pending'",
                [],
            )
            .map_err(err)?;
        let paused = self
            .conn
            .execute(
                "UPDATE runs SET paused = 1 WHERE execution_state = 'queued'",
                [],
            )
            .map_err(err)?;
        Ok(CrashRecovery {
            interrupted,
            expired_approvals: expired,
            paused_queued: paused,
        })
    }

    pub fn unpause_queued(&self) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE runs SET paused = 0 WHERE execution_state = 'queued'",
                [],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn insert_attachment(&self, att: &Attachment) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO attachments (id, original_name, mime, size, stored_path, message_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    att.id,
                    att.original_name,
                    att.mime,
                    att.size,
                    att.stored_path,
                    att.message_id
                ],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn get_attachment(&self, id: &str) -> Result<Attachment, String> {
        self.conn
            .query_row(
                "SELECT id, original_name, mime, size, stored_path, message_id
                 FROM attachments WHERE id = ?1",
                [id],
                |row| {
                    Ok(Attachment {
                        id: row.get(0)?,
                        original_name: row.get(1)?,
                        mime: row.get(2)?,
                        size: row.get(3)?,
                        stored_path: row.get(4)?,
                        message_id: row.get(5)?,
                    })
                },
            )
            .map_err(err)
    }

    pub fn list_attachments_for_message(&self, message_id: &str) -> Result<Vec<Attachment>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, original_name, mime, size, stored_path, message_id
                 FROM attachments WHERE message_id = ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([message_id], |row| {
                Ok(Attachment {
                    id: row.get(0)?,
                    original_name: row.get(1)?,
                    mime: row.get(2)?,
                    size: row.get(3)?,
                    stored_path: row.get(4)?,
                    message_id: row.get(5)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)
    }

    pub fn list_runs_by_state(&self, state: &str) -> Result<Vec<Run>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, message_id, execution_state, error, request_id, paused, created_at
                 FROM runs WHERE execution_state = ?1 ORDER BY created_at, id",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([state], row_to_run)
            .map_err(err)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(err)
    }

    pub fn bind_attachment(&self, id: &str, message_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE attachments SET message_id = ?1 WHERE id = ?2",
                params![message_id, id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn insert_notification(&self, n: &Notification) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO notifications
                 (id, session_id, run_id, kind, priority, title, unread, dedupe_key, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    n.id,
                    n.session_id,
                    n.run_id,
                    n.kind,
                    n.priority,
                    n.title,
                    n.unread as i64,
                    n.dedupe_key,
                    n.created_at
                ],
            )
            .map_err(err)?;
        Ok(changed > 0)
    }

    pub fn list_notifications(&self, session_id: Option<&str>) -> Result<Vec<Notification>, String> {
        let sql = if session_id.is_some() {
            "SELECT id, session_id, run_id, kind, priority, title, unread, dedupe_key, created_at
             FROM notifications WHERE session_id = ?1
             ORDER BY priority, created_at, id"
        } else {
            "SELECT id, session_id, run_id, kind, priority, title, unread, dedupe_key, created_at
             FROM notifications ORDER BY priority, created_at, id"
        };
        let mut stmt = self.conn.prepare(sql).map_err(err)?;
        let map = |row: &rusqlite::Row| {
            Ok(Notification {
                id: row.get(0)?,
                session_id: row.get(1)?,
                run_id: row.get(2)?,
                kind: row.get(3)?,
                priority: row.get(4)?,
                title: row.get(5)?,
                unread: row.get::<_, i64>(6)? != 0,
                dedupe_key: row.get(7)?,
                created_at: row.get(8)?,
            })
        };
        if let Some(id) = session_id {
            stmt.query_map([id], map)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)
        } else {
            stmt.query_map([], map)
                .map_err(err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(err)
        }
    }

    pub fn insert_approval(
        &self,
        id: &str,
        run_id: &str,
        provider_request_id: &str,
        status: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO approvals (id, run_id, provider_request_id, status)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, run_id, provider_request_id, status],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn pending_approval_for_run(&self, run_id: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT id FROM approvals WHERE run_id = ?1 AND status = 'pending' LIMIT 1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(err)
    }

    pub fn set_approval_status(&self, id: &str, status: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE approvals SET status = ?1 WHERE id = ?2",
                params![status, id],
            )
            .map_err(err)?;
        Ok(())
    }

    pub fn expire_approvals_for_run(&self, run_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE approvals SET status = 'expired' WHERE run_id = ?1 AND status = 'pending'",
                [run_id],
            )
            .map_err(err)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecovery {
    pub interrupted: usize,
    pub expired_approvals: usize,
    pub paused_queued: usize,
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        team_id: row.get(1)?,
        name: row.get(2)?,
        provider: row.get(3)?,
        workspace_path: row.get(4)?,
        cli_session_id: row.get(5)?,
        avatar: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
        model: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        role_description: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
        created_at: row.get::<_, Option<i64>>(9)?.unwrap_or(0),
    })
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    Ok(Message {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        request_id: row.get(5)?,
        created_at: row.get(6)?,
        attachments: Vec::new(),
    })
}

fn row_to_run(row: &rusqlite::Row) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        session_id: row.get(1)?,
        message_id: row.get(2)?,
        execution_state: row.get(3)?,
        error: row.get(4)?,
        request_id: row.get(5)?,
        paused: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
    })
}

fn err(e: impl ToString) -> String {
    e.to_string()
}
