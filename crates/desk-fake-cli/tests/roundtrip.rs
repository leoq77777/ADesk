use desk_core::{AcpClient, ApprovalDecision, ClientEvent, EventAction};
use std::path::PathBuf;

fn fake_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_desk-fake-cli"))
}

fn temp_cwd() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "desk-m1-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test]
async fn p01_reply_round() {
    let cwd = temp_cwd();
    let mut client = AcpClient::spawn(&fake_bin(), &cwd).await.unwrap();
    client.initialize().await.unwrap();
    let sid = client.session_new(cwd.to_str().unwrap()).await.unwrap();
    assert!(sid.starts_with("fake-"));
    let out = client
        .prompt(&sid, "hello", |_| EventAction::Continue)
        .await
        .unwrap();
    assert_eq!(out.text, "echo:hello");
    assert_eq!(out.stop_reason, "end_turn");
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn u01_session_load_keeps_id() {
    let cwd = temp_cwd();
    let mut client = AcpClient::spawn(&fake_bin(), &cwd).await.unwrap();
    client.initialize().await.unwrap();
    client
        .session_load("employee-a", cwd.to_str().unwrap())
        .await
        .unwrap();
    let out = client
        .prompt("employee-a", "ping", |_| EventAction::Continue)
        .await
        .unwrap();
    assert_eq!(out.text, "echo:ping");
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn u02_prompt_keeps_quotes_and_cjk() {
    let cwd = temp_cwd();
    let mut client = AcpClient::spawn(&fake_bin(), &cwd).await.unwrap();
    client.initialize().await.unwrap();
    let sid = client.session_new(cwd.to_str().unwrap()).await.unwrap();
    let prompt = "say \"hi\" and 中文; rm -rf /";
    let out = client
        .prompt(&sid, prompt, |_| EventAction::Continue)
        .await
        .unwrap();
    assert_eq!(out.text, format!("echo:{prompt}"));
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn p11_p12_approval_then_continue() {
    let cwd = temp_cwd();
    let mut client = AcpClient::spawn_args(&fake_bin(), &cwd, &["--scenario", "approve"])
        .await
        .unwrap();
    client.initialize().await.unwrap();
    let sid = client.session_new(cwd.to_str().unwrap()).await.unwrap();
    let mut saw_approval = false;
    let out = client
        .prompt(&sid, "uname", |ev| match ev {
            ClientEvent::ApprovalRequested { tool, .. } => {
                saw_approval = true;
                assert_eq!(tool, "uname");
                EventAction::Approve(ApprovalDecision::AllowOnce)
            }
            _ => EventAction::Continue,
        })
        .await
        .unwrap();
    assert!(saw_approval);
    assert_eq!(out.text, "approved:uname");
    client.shutdown().await.unwrap();
}

#[tokio::test]
async fn p06_cancel_hanging_prompt() {
    let cwd = temp_cwd();
    let mut client = AcpClient::spawn_args(&fake_bin(), &cwd, &["--scenario", "hang"])
        .await
        .unwrap();
    client.initialize().await.unwrap();
    let sid = client.session_new(cwd.to_str().unwrap()).await.unwrap();
    let out = client
        .prompt(&sid, "long", |ev| match ev {
            ClientEvent::Barrier { phase } if phase == "hanging" => EventAction::Cancel,
            _ => EventAction::Continue,
        })
        .await
        .unwrap();
    assert_eq!(out.stop_reason, "cancelled");
    assert_eq!(out.text, "cancelled");
    client.shutdown().await.unwrap();
}
