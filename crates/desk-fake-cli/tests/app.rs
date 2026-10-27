use std::fs;
use std::path::PathBuf;

use desk_core::{App, ApprovalDecision, Provider};

fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_desk-fake-cli"))
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "desk-m2-{label}-{}-{}",
        std::process::id(),
        uuid_like()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn uuid_like() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn open_app(root: &std::path::Path) -> App {
    App::open(
        root.join("desk.db"),
        Provider::Fake {
            bin: fake_bin(),
            scenario: "reply".into(),
        },
    )
    .unwrap()
}

#[tokio::test]
async fn two_employees_isolated() {
    let root = temp_dir("two");
    let ws_a = root.join("ada");
    let ws_b = root.join("ben");
    fs::create_dir_all(&ws_a).unwrap();
    fs::create_dir_all(&ws_b).unwrap();

    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let ada = app.create_session(&team.id, "ada", &ws_a).unwrap();
    let ben = app.create_session(&team.id, "ben", &ws_b).unwrap();

    let a = app.submit(&ada.id, "alpha").await.unwrap();
    let b = app.submit(&ben.id, "beta").await.unwrap();
    assert_eq!(a.reply, "echo:alpha");
    assert_eq!(b.reply, "echo:beta");

    let ada_msgs = app.list_messages(&ada.id).unwrap();
    let ben_msgs = app.list_messages(&ben.id).unwrap();
    assert!(ada_msgs.iter().any(|m| m.content == "alpha"));
    assert!(ada_msgs.iter().any(|m| m.content == "echo:alpha"));
    assert!(!ada_msgs.iter().any(|m| m.content.contains("beta")));
    assert!(ben_msgs.iter().any(|m| m.content == "beta"));
    assert!(!ben_msgs.iter().any(|m| m.content.contains("alpha")));

    let ada_cli = app.get_session(&ada.id).unwrap().cli_session_id;
    let ben_cli = app.get_session(&ben.id).unwrap().cli_session_id;
    assert!(ada_cli.is_some());
    assert_ne!(ada_cli, ben_cli);
}

#[tokio::test]
async fn failed_run_syncs_cli_error_into_chat() {
    let root = temp_dir("fail-sync");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let err = app.submit(&session.id, "FAIL").await.unwrap_err();
    assert!(err.contains("quota exceeded"), "{err}");
    assert!(err.contains("missing API key") || err.contains("quota"), "{err}");
    let msgs = app.list_messages(&session.id).unwrap();
    let error = msgs
        .iter()
        .find(|m| m.role == "error")
        .expect("error message");
    assert!(error.content.contains("quota exceeded"), "{}", error.content);
    let snap = app.snapshot().unwrap();
    assert!(
        snap.notifications.iter().any(|n| n.title.contains("quota") || n.title.contains("error")),
        "{:?}",
        snap.notifications
    );
}

#[tokio::test]
async fn resume_by_cli_session_id() {
    let root = temp_dir("resume");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();

    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    app.submit(&session.id, "marker-42").await.unwrap();
    let cli_id = app
        .get_session(&session.id)
        .unwrap()
        .cli_session_id
        .expect("cli session persisted");

    app.detach(&session.id).await.unwrap();
    let out = app.submit(&session.id, "RECALL").await.unwrap();
    assert_eq!(out.reply, "echo:marker-42");
    assert_eq!(
        app.get_session(&session.id).unwrap().cli_session_id.as_deref(),
        Some(cli_id.as_str())
    );

    let msgs = app.list_messages(&session.id).unwrap();
    assert_eq!(msgs.len(), 4);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[0].content, "marker-42");
}

#[tokio::test]
async fn stale_cli_session_is_recreated() {
    let root = temp_dir("stale-cli");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    app.submit(&session.id, "hello").await.unwrap();
    app.set_cli_session_id(&session.id, "missing-old-shift")
        .unwrap();
    app.detach(&session.id).await.unwrap();
    let out = app.submit(&session.id, "back").await.unwrap();
    assert_eq!(out.reply, "echo:back");
    let cli = app
        .get_session(&session.id)
        .unwrap()
        .cli_session_id
        .expect("new cli session");
    assert!(!cli.starts_with("missing-"), "{cli}");
}

#[tokio::test]
async fn live_stale_cli_session_is_recreated() {
    let root = temp_dir("live-stale-cli");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    app.submit(&session.id, "hello").await.unwrap();
    app.set_cli_session_id(&session.id, "missing-live-shift")
        .unwrap();
    let out = app.submit(&session.id, "back").await.unwrap();
    assert_eq!(out.reply, "echo:back");
    let cli = app
        .get_session(&session.id)
        .unwrap()
        .cli_session_id
        .expect("new cli session");
    assert!(!cli.starts_with("missing-"), "{cli}");
}

#[tokio::test]
async fn clock_in_opens_fresh_cli_session() {
    let root = temp_dir("clock-in");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    app.submit(&session.id, "hello").await.unwrap();
    app.set_cli_session_id(&session.id, "missing-clock")
        .unwrap();
    let view = app.clock_in(&session.id).await.unwrap();
    assert_eq!(view.seat, "idle");
    let cli = view.cli_session_id.expect("fresh cli session");
    assert!(!cli.starts_with("missing-"), "{cli}");
    let msgs = app.list_messages(&session.id).unwrap();
    assert_eq!(msgs.last().map(|m| m.content.as_str()), Some("已重新到岗。"));
    assert!(app
        .snapshot()
        .unwrap()
        .notifications
        .iter()
        .all(|n| n.kind != "replied"));
}

async fn wait_state(app: &mut App, run_id: &str, state: &str) {
    for _ in 0..80 {
        app.tick().await.unwrap();
        if app.get_run(run_id).unwrap().execution_state == state {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "run {run_id} stayed {} wanted {state}",
        app.get_run(run_id).unwrap().execution_state
    );
}

#[tokio::test]
async fn p09_two_hang_third_waits_for_slot() {
    let root = temp_dir("p09");
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let sessions: Vec<_> = (0..3)
        .map(|i| {
            let ws = root.join(format!("w{i}"));
            fs::create_dir_all(&ws).unwrap();
            app.create_session(&team.id, &format!("e{i}"), &ws).unwrap()
        })
        .collect();

    let r1 = app.enqueue(&sessions[0].id, "HANG", None, &[]).unwrap();
    let r2 = app.enqueue(&sessions[1].id, "HANG", None, &[]).unwrap();
    let r3 = app.enqueue(&sessions[2].id, "hello", None, &[]).unwrap();
    app.tick().await.unwrap();
    wait_state(&mut app, &r1, "running").await;
    wait_state(&mut app, &r2, "running").await;
    assert_eq!(app.active_count().unwrap(), 2);
    assert!(app.queued_run_ids().unwrap().contains(&r3));
    assert_eq!(app.get_run(&r3).unwrap().execution_state, "queued");

    app.cancel_run(&r1).await.unwrap();
    let out = app.wait_run(&r3).await.unwrap();
    assert_eq!(out.reply, "echo:hello");
    let _ = app.cancel_run(&r2).await;
}

#[tokio::test]
async fn attachment_reads_snapshot_not_source() {
    let root = temp_dir("att");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let src = root.join("note.txt");
    fs::write(&src, "secret-xyz").unwrap();

    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let att = app.stage_attachment(&src).unwrap();
    fs::write(&src, "changed-after-stage").unwrap();
    let run_id = app
        .enqueue(&session.id, "read file", None, &[&att.id])
        .unwrap();
    let out = app.wait_run(&run_id).await.unwrap();
    assert_eq!(out.reply, "file:secret-xyz");
}

#[tokio::test]
async fn recover_interrupts_running_and_pauses_queue() {
    let root = temp_dir("crash");
    let db = root.join("desk.db");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let team_id;
    let hang_id;
    let queued_id;
    {
        let mut app = open_app(&root);
        let team = app.create_team("lab").unwrap();
        team_id = team.id.clone();
        let session = app.create_session(&team.id, "ada", &ws).unwrap();
        hang_id = app.enqueue(&session.id, "HANG", None, &[]).unwrap();
        queued_id = app.enqueue(&session.id, "hello", None, &[]).unwrap();
        app.tick().await.unwrap();
        wait_state(&mut app, &hang_id, "running").await;
    }

    let mut app = open_app(&root);
    assert!(app.dispatch_paused());
    assert_eq!(app.get_run(&hang_id).unwrap().execution_state, "interrupted");
    let queued = app.get_run(&queued_id).unwrap();
    assert_eq!(queued.execution_state, "queued");
    assert!(queued.paused);
    assert_eq!(db, root.join("desk.db"));
    assert!(!team_id.is_empty());

    app.resume_queue().unwrap();
    let out = app.wait_run(&queued_id).await.unwrap();
    assert_eq!(out.reply, "echo:hello");
    let notes = app.list_notifications(None).unwrap();
    assert!(notes.iter().any(|n| n.kind == "replied"));
}

#[tokio::test]
async fn request_id_is_idempotent() {
    let root = temp_dir("idemp");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let a = app
        .enqueue(&session.id, "once", Some("req-1"), &[])
        .unwrap();
    let b = app
        .enqueue(&session.id, "once", Some("req-1"), &[])
        .unwrap();
    assert_eq!(a, b);
    let conflict = app.enqueue(&session.id, "other", Some("req-1"), &[]);
    assert!(conflict.unwrap_err().contains("conflict"));
    let out = app.wait_run(&a).await.unwrap();
    assert_eq!(out.reply, "echo:once");
}

#[tokio::test]
async fn approval_waits_until_resolved() {
    let root = temp_dir("approve");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let run_id = app.enqueue(&session.id, "APPROVE", None, &[]).unwrap();
    app.tick().await.unwrap();
    for _ in 0..80 {
        if app
            .list_notifications(None)
            .unwrap()
            .iter()
            .any(|n| n.kind == "approval")
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(app
        .list_notifications(None)
        .unwrap()
        .iter()
        .any(|n| n.kind == "approval"));
    assert_eq!(app.get_run(&run_id).unwrap().execution_state, "running");
    app.resolve_approval(&run_id, ApprovalDecision::AllowOnce)
        .unwrap();
    let out = app.wait_run(&run_id).await.unwrap();
    assert_eq!(out.reply, "approved:APPROVE");
}

#[test]
fn second_instance_cannot_open_same_db() {
    let root = temp_dir("lock");
    let _first = open_app(&root);
    let second = App::open(
        root.join("desk.db"),
        Provider::Fake {
            bin: fake_bin(),
            scenario: "reply".into(),
        },
    );
    match second {
        Ok(_) => panic!("second instance should be rejected"),
        Err(err) => assert!(err.contains("another Desk instance"), "{err}"),
    }
}

#[tokio::test]
async fn stop_and_replace_runs_before_older_queue() {
    let root = temp_dir("replace");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let hang = app.enqueue(&session.id, "HANG", None, &[]).unwrap();
    let queued = app.enqueue(&session.id, "hello", None, &[]).unwrap();
    app.tick().await.unwrap();
    wait_state(&mut app, &hang, "running").await;
    let replacement = app.stop_and_replace(&session.id, "new").await.unwrap();
    let out = app.wait_run(&replacement).await.unwrap();
    assert_eq!(out.reply, "echo:new");
    let later = app.wait_run(&queued).await.unwrap();
    assert_eq!(later.reply, "echo:hello");
}

#[tokio::test]
async fn snapshot_lists_team_and_seat() {
    let root = temp_dir("snap");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    app.submit(&session.id, "hi").await.unwrap();
    let snap = app.snapshot().unwrap();
    assert_eq!(snap.teams.len(), 1);
    assert_eq!(snap.sessions[0].name, "ada");
    assert_eq!(snap.sessions[0].seat, "idle");
}

#[test]
fn create_session_empty_workspace_makes_dir() {
    let root = temp_dir("ws-auto");
    let app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "Ada", "").unwrap();
    assert!(std::path::Path::new(&session.workspace_path).is_dir());
    assert!(session.workspace_path.contains("workspaces"));
}

#[test]
fn create_session_rejects_unknown_provider() {
    let root = temp_dir("prov");
    let app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let err = app
        .create_session_with(&team.id, "ada", root.join("ws"), Some("nope"))
        .unwrap_err();
    assert!(err.contains("unknown provider"), "{err}");
}

#[tokio::test]
async fn probe_fake_is_available() {
    let root = temp_dir("probe-ok");
    let mut app = open_app(&root);
    let probes = app.probe_providers().await.unwrap();
    let fake = probes.iter().find(|p| p.name == "fake").expect("fake");
    assert!(fake.available, "{}", fake.detail);
    let snap = app.snapshot().unwrap();
    assert!(snap.probes.iter().any(|p| p.name == "fake" && p.available));
}

#[tokio::test]
async fn probe_discovers_clis_installed_on_machine() {
    let root = temp_dir("probe-scan");
    let mut app = open_app(&root);
    let probes = app.probe_providers().await.unwrap();
    if desk_core::catalog::find_executable("", &["gemini"]).is_some() {
        assert!(
            probes.iter().any(|p| p.name == "gemini"),
            "gemini installed but not probed: {probes:?}"
        );
    }
    if desk_core::catalog::find_executable("", &["codex"]).is_some() {
        assert!(
            probes.iter().any(|p| p.name == "codex"),
            "codex installed but not probed: {probes:?}"
        );
    }
}

#[tokio::test]
async fn probe_codex_without_adapter_explains_acp() {
    let root = temp_dir("probe-codex");
    std::env::set_var("DESK_CODEX_ACP", root.join("missing-codex-acp"));
    let mut app = open_app(&root);
    let probes = app.probe_providers().await.unwrap();
    std::env::remove_var("DESK_CODEX_ACP");
    let codex = probes.iter().find(|p| p.name == "codex").expect("codex");
    assert!(!codex.available);
    assert!(
        codex.detail.contains("ACP") || codex.detail.contains("适配器"),
        "{}",
        codex.detail
    );
    assert_eq!(
        codex.adapter_package.as_deref(),
        Some("@agentclientprotocol/codex-acp")
    );
}

#[tokio::test]
async fn probe_missing_bin_blocks_create() {
    let root = temp_dir("probe-miss");
    let mut app = App::open(
        root.join("desk.db"),
        Provider::Fake {
            bin: root.join("no-such-fake"),
            scenario: "reply".into(),
        },
    )
    .unwrap();
    let probes = app.probe_providers().await.unwrap();
    let fake = probes.iter().find(|p| p.name == "fake").expect("fake");
    assert!(!fake.available);
    assert!(fake.detail.contains("未找到"), "{}", fake.detail);
    let team = app.create_team("lab").unwrap();
    let err = app.create_session(&team.id, "ada", "").unwrap_err();
    assert!(err.contains("不可用"), "{err}");
}

#[tokio::test]
async fn create_session_ok_after_successful_probe() {
    let root = temp_dir("probe-create");
    let mut app = open_app(&root);
    app.probe_providers().await.unwrap();
    let team = app.create_team("lab").unwrap();
    let session = app
        .create_session_with(&team.id, "ada", "", Some("fake"))
        .unwrap();
    assert_eq!(session.provider, "fake");
}

#[tokio::test]
async fn stage_bytes_reads_like_file() {
    let root = temp_dir("bytes");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let att = app
        .stage_bytes("note.txt", "text/plain", b"secret-xyz")
        .unwrap();
    let run_id = app
        .enqueue(&session.id, "read file", None, &[&att.id])
        .unwrap();
    let out = app.wait_run(&run_id).await.unwrap();
    assert_eq!(out.reply, "file:secret-xyz");
}

#[tokio::test]
async fn list_messages_page_walks_backward() {
    let root = temp_dir("page");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    for text in ["one", "two", "three"] {
        app.submit(&session.id, text).await.unwrap();
    }
    let all = app.list_messages(&session.id).unwrap();
    assert_eq!(all.len(), 6);
    let last = app
        .list_messages_page(&session.id, None, None, 2)
        .unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[0].id, all[4].id);
    assert_eq!(last[1].id, all[5].id);
    let older = app
        .list_messages_page(&session.id, Some(last[0].created_at), Some(&last[0].id), 2)
        .unwrap();
    assert_eq!(older.len(), 2);
    assert_eq!(older[0].id, all[2].id);
    assert_eq!(older[1].id, all[3].id);
}

#[tokio::test]
async fn f01_snapshot_and_notification_point_at_session() {
    let root = temp_dir("f01");
    let ws_a = root.join("ada");
    let ws_b = root.join("ben");
    fs::create_dir_all(&ws_a).unwrap();
    fs::create_dir_all(&ws_b).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let ada = app.create_session(&team.id, "ada", &ws_a).unwrap();
    let ben = app.create_session(&team.id, "ben", &ws_b).unwrap();
    app.submit(&ada.id, "alpha").await.unwrap();
    app.submit(&ben.id, "beta").await.unwrap();
    let snap = app.snapshot().unwrap();
    assert_eq!(snap.teams.len(), 1);
    assert_eq!(snap.sessions.len(), 2);
    let notes = app.list_notifications(None).unwrap();
    let ada_note = notes
        .iter()
        .find(|n| n.session_id == ada.id)
        .expect("ada notice");
    assert_eq!(ada_note.kind, "replied");
    app.mark_read(&ada_note.id).unwrap();
    assert!(!app
        .list_notifications(None)
        .unwrap()
        .iter()
        .find(|n| n.id == ada_note.id)
        .unwrap()
        .unread);
    let ada_msgs = app.list_messages(&ada.id).unwrap();
    let ben_msgs = app.list_messages(&ben.id).unwrap();
    assert!(ada_msgs.iter().all(|m| !m.content.contains("beta")));
    assert!(ben_msgs.iter().all(|m| !m.content.contains("alpha")));
}

#[tokio::test]
async fn active_run_kept_in_snapshot_when_followup_queued() {
    let root = temp_dir("active-plus-queued");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let hang = app.enqueue(&session.id, "HANG", None, &[]).unwrap();
    app.tick().await.unwrap();
    wait_state(&mut app, &hang, "running").await;

    let _queued = app.enqueue(&session.id, "followup", None, &[]).unwrap();
    let snap = app.snapshot().unwrap();
    let view = &snap.sessions[0];
    assert_eq!(view.seat, "working");
    assert_eq!(view.run_id.as_deref(), Some(hang.as_str()));
    assert_eq!(view.queued_count, 1);

    let _ = app.cancel_run(&hang).await.unwrap();
}

#[tokio::test]
async fn cancel_during_approval_clears_awaiting_approval_seat() {
    let root = temp_dir("cancel-approve");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "ada", &ws).unwrap();
    let run_id = app.enqueue(&session.id, "APPROVE", None, &[]).unwrap();
    app.tick().await.unwrap();
    for _ in 0..80 {
        if app.snapshot().unwrap().sessions[0].seat == "awaiting_approval" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(app.snapshot().unwrap().sessions[0].seat, "awaiting_approval");
    app.cancel_run(&run_id).await.unwrap();
    assert_eq!(app.snapshot().unwrap().sessions[0].seat, "idle");
}

#[tokio::test]
async fn clock_in_falls_back_when_session_provider_is_broken() {
    let root = temp_dir("clock-in-fallback");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let team = app.create_team("lab").unwrap();
    let session = app.create_session(&team.id, "leo", &ws).unwrap();
    // Register a broken custom provider whose binary does not exist
    app.register_provider(Provider::acp(
        "broken-cli",
        root.join("missing-cli-bin"),
        &[],
        None,
    ));
    app.set_session_provider(&session.id, "broken-cli").unwrap();
    let view = app.clock_in(&session.id).await.unwrap();
    assert_eq!(view.seat, "idle");
    assert_eq!(view.provider, "fake");
}

#[tokio::test]
async fn update_profile_and_delete_faulty_session() {
    let root = temp_dir("profile-delete");
    let ws = root.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let mut app = open_app(&root);
    let probes = app.probe_providers().await.unwrap();
    let fake_probe = probes.iter().find(|p| p.name == "fake").expect("fake probe");
    assert_eq!(fake_probe.default_model.as_deref(), Some("fake-default"));
    assert!(fake_probe.models.iter().any(|m| m.id == "fake-fast"));

    let team = app.create_team("lab").unwrap();
    // Invalid model for probed provider is rejected at creation time
    let bad = app.create_session_profile(
        &team.id,
        "BadModel",
        &ws,
        Some("fake"),
        Some("non-existent-model"),
        None,
        None,
    );
    assert!(bad.is_err());

    let session = app
        .create_session_profile(
            &team.id,
            "Ada",
            &ws,
            Some("fake"),
            Some("fake-fast"),
            Some("核心架构与后端开发"),
            Some("maru"),
        )
        .unwrap();
    assert_eq!(session.model, "fake-fast");
    assert_eq!(session.role_description, "核心架构与后端开发");
    assert_eq!(session.avatar, "maru");
    assert!(session.created_at > 0);

    // Model is locked after creation: update_session_profile keeps original model
    let updated = app
        .update_session_profile(
            &session.id,
            "Ada Lovelace",
            "fake",
            "fake-default",
            "自动化测试与故障排查",
            "abigail",
        )
        .await
        .unwrap();
    assert_eq!(updated.name, "Ada Lovelace");
    assert_eq!(updated.model, "fake-fast");
    assert_eq!(updated.role_description, "自动化测试与故障排查");
    assert_eq!(updated.avatar, "abigail");

    let _ = app.submit(&session.id, "FAIL").await;
    assert_eq!(app.snapshot().unwrap().sessions[0].seat, "error");

    app.delete_session(&session.id).await.unwrap();
    assert!(app.snapshot().unwrap().sessions.is_empty());
}



