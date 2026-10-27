use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use desk_core::{AcpClient, App, ApprovalDecision, ClientEvent, EventAction, Provider};

#[tokio::main]
async fn main() -> ExitCode {
    let mut scenario = "reply".to_owned();
    let mut prompt = "hello from harness".to_owned();
    let mut provider = "fake".to_owned();
    let mut demo = String::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => scenario = args.next().unwrap_or(scenario),
            "--prompt" => prompt = args.next().unwrap_or(prompt),
            "--provider" => provider = args.next().unwrap_or(provider),
            "--demo" => demo = args.next().unwrap_or(demo),
            _ => {}
        }
    }

    if !demo.is_empty() {
        return match run_demo(&demo, &provider, &scenario).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("{err}");
                ExitCode::from(1)
            }
        };
    }

    let cwd = std::env::temp_dir().join(format!("desk-harness-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&cwd);
    if provider == "cursor" {
        let _ = std::process::Command::new("git").arg("init").current_dir(&cwd).status();
    }

    let mut client = match spawn_provider(&provider, &scenario, &cwd).await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };

    let run = run(&mut client, &cwd, &prompt, provider == "cursor");
    let result = if provider == "cursor" {
        tokio::time::timeout(Duration::from_secs(90), run).await
    } else {
        Ok(run.await)
    };
    match result {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            eprintln!("{err}");
            let _ = client.shutdown().await;
            return ExitCode::from(1);
        }
        Err(_) => {
            eprintln!("cursor ACP prompt timed out");
            let _ = client.shutdown().await;
            return ExitCode::from(1);
        }
    }
    let _ = client.shutdown().await;
    ExitCode::SUCCESS
}

async fn run_demo(demo: &str, provider: &str, scenario: &str) -> Result<(), String> {
    let root = std::env::temp_dir().join(format!("desk-demo-{}-{}", demo, std::process::id()));
    let _ = std::fs::create_dir_all(&root);
    let kind = match provider {
        "cursor" => Provider::cursor(
            cursor_bin().ok_or("agent not found; set DESK_CURSOR_AGENT")?,
        ),
        _ => Provider::Fake {
            bin: fake_cli_bin(),
            scenario: scenario.to_owned(),
        },
    };
    let mut app = App::open(root.join("desk.db"), kind)?;
    let team = app.create_team("lab")?;
    match demo {
        "two" => {
            let ada_ws = root.join("ada");
            let ben_ws = root.join("ben");
            std::fs::create_dir_all(&ada_ws).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&ben_ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ada_ws).status();
                let _ = std::process::Command::new("git").arg("init").current_dir(&ben_ws).status();
            }
            let ada = app.create_session(&team.id, "ada", &ada_ws)?;
            let ben = app.create_session(&team.id, "ben", &ben_ws)?;
            let a = app.submit(&ada.id, "alpha").await?;
            let b = app.submit(&ben.id, "beta").await?;
            println!("ada.session={}", ada.id);
            println!("ada.cli={:?}", app.get_session(&ada.id)?.cli_session_id);
            println!("ada.reply={}", a.reply);
            println!("ben.session={}", ben.id);
            println!("ben.cli={:?}", app.get_session(&ben.id)?.cli_session_id);
            println!("ben.reply={}", b.reply);
        }
        "resume" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ws).status();
            }
            let session = app.create_session(&team.id, "ada", &ws)?;
            let (first_prompt, second_prompt) = if provider == "cursor" {
                (
                    "Remember the marker MARK-42. Reply with exactly OK.",
                    "What marker did I ask you to remember? Reply with only that marker.",
                )
            } else {
                ("marker-42", "RECALL")
            };
            let first = app.submit(&session.id, first_prompt).await?;
            let cli = app.get_session(&session.id)?.cli_session_id;
            app.detach(&session.id).await?;
            let second = app.submit(&session.id, second_prompt).await?;
            println!("session={}", session.id);
            println!("cli={cli:?}");
            println!("first={}", first.reply);
            println!("recall={}", second.reply);
            println!("messages={}", app.list_messages(&session.id)?.len());
        }
        "attach" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ws).status();
            }
            let src = root.join("note.txt");
            std::fs::write(&src, "secret-xyz").map_err(|e| e.to_string())?;
            let session = app.create_session(&team.id, "ada", &ws)?;
            let att = app.stage_attachment(&src)?;
            std::fs::write(&src, "changed").map_err(|e| e.to_string())?;
            let prompt = if provider == "cursor" {
                "The file is attached. Reply with only its exact contents, nothing else."
            } else {
                "read file"
            };
            let run = app.enqueue(&session.id, prompt, None, &[&att.id])?;
            let out = app.wait_run(&run).await?;
            println!("reply={}", out.reply);
        }
        "schedule" => {
            let sessions: Result<Vec<_>, String> = (0..3)
                .map(|i| {
                    let ws = root.join(format!("w{i}"));
                    std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
                    app.create_session(&team.id, &format!("e{i}"), &ws)
                })
                .collect();
            let sessions = sessions?;
            let r1 = app.enqueue(&sessions[0].id, "HANG", None, &[])?;
            let r2 = app.enqueue(&sessions[1].id, "HANG", None, &[])?;
            let r3 = app.enqueue(&sessions[2].id, "hello", None, &[])?;
            app.tick().await?;
            println!("active={}", app.active_count()?);
            println!("queued={:?}", app.queued_run_ids()?);
            let _ = app.cancel_run(&r1).await?;
            let out = app.wait_run(&r3).await?;
            println!("third={}", out.reply);
            let _ = app.cancel_run(&r2).await;
        }
        "approve" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            let session = app.create_session(&team.id, "ada", &ws)?;
            let run = app.enqueue(&session.id, "APPROVE", None, &[])?;
            app.tick().await?;
            for _ in 0..80 {
                if app
                    .list_notifications(None)?
                    .iter()
                    .any(|n| n.kind == "approval")
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            app.resolve_approval(&run, ApprovalDecision::AllowOnce)?;
            let out = app.wait_run(&run).await?;
            println!("reply={}", out.reply);
        }
        "image" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ws).status();
            }
            let png = image_fixture()?;
            let session = app.create_session(&team.id, "ada", &ws)?;
            let att = app.stage_attachment(&png)?;
            let prompt = if provider == "cursor" {
                "Inspect the attached image only. After stripping leading and trailing whitespace, report the exact text drawn in the image, then list each shape kind and how many. Valid kinds: circle, triangle, square. Do not infer from this prompt. Reply as JSON {\"text\":\"...\",\"shapes\":[{\"kind\":\"...\",\"count\":1}]}"
            } else {
                "see image"
            };
            let run = app.enqueue(&session.id, prompt, None, &[&att.id])?;
            let out = app.wait_run(&run).await?;
            println!("reply={}", out.reply);
        }
        "cancel" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ws).status();
            }
            let session = app.create_session(&team.id, "ada", &ws)?;
            let prompt = if provider == "cursor" {
                "Write 80 numbered lines of the word tick, one per line. Do not stop early."
            } else {
                "HANG"
            };
            let run = app.enqueue(&session.id, prompt, None, &[])?;
            app.tick().await?;
            for _ in 0..80 {
                if app.get_run(&run)?.execution_state == "running" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            if provider == "cursor" {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            let out = app.cancel_run(&run).await?;
            println!("cancel={}", out);
            println!("state={}", app.get_run(&run)?.execution_state);
            let again = app.submit(&session.id, "Reply with exactly the word pong.").await?;
            println!("after={}", again.reply);
        }
        "write" => {
            let ws = root.join("ws");
            std::fs::create_dir_all(&ws).map_err(|e| e.to_string())?;
            if provider == "cursor" {
                let _ = std::process::Command::new("git").arg("init").current_dir(&ws).status();
            }
            let session = app.create_session(&team.id, "ada", &ws)?;
            let run = app.enqueue(
                &session.id,
                "Run `uname` and write acp-hello.txt with hi.",
                None,
                &[],
            )?;
            app.tick().await?;
            for _ in 0..120 {
                let notes = app.list_notifications(None)?;
                if notes.iter().any(|n| n.kind == "approval") {
                    println!("approval=pending");
                    app.resolve_approval(&run, ApprovalDecision::AllowOnce)?;
                    break;
                }
                if matches!(
                    app.get_run(&run)?.execution_state.as_str(),
                    "succeeded" | "failed" | "cancelled"
                ) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let out = app.wait_run(&run).await?;
            let path = ws.join("acp-hello.txt");
            println!("reply={}", out.reply);
            println!("file={}", path.exists());
            if path.exists() {
                println!("contents={}", std::fs::read_to_string(&path).unwrap_or_default());
            }
            println!(
                "approvals={}",
                app.list_notifications(None)?
                    .iter()
                    .filter(|n| n.kind == "approval")
                    .count()
            );
        }
        other => return Err(format!("unknown demo: {other}")),
    }
    Ok(())
}

async fn spawn_provider(provider: &str, scenario: &str, cwd: &Path) -> Result<AcpClient, String> {
    match provider {
        "cursor" => {
            let agent = cursor_bin().ok_or("agent not found; set DESK_CURSOR_AGENT")?;
            let mut client = AcpClient::spawn_args(&agent, cwd, &["acp"])
                .await
                .map_err(|e| format!("spawn agent acp: {e}"))?;
            client.initialize().await.map_err(|e| format!("initialize: {e}"))?;
            client
                .authenticate("cursor_login")
                .await
                .map_err(|e| format!("authenticate: {e}"))?;
            Ok(client)
        }
        _ => {
            let fake = fake_cli_bin();
            AcpClient::spawn_args(&fake, cwd, &["--scenario", scenario])
                .await
                .map_err(|e| format!("spawn fake cli: {e}"))
        }
    }
}

async fn run(client: &mut AcpClient, cwd: &Path, prompt: &str, already_init: bool) -> Result<(), String> {
    if !already_init {
        client.initialize().await.map_err(|e| e.to_string())?;
    }
    let sid = client
        .session_new(cwd.to_str().unwrap_or("."))
        .await
        .map_err(|e| e.to_string())?;
    let mut approvals = 0usize;
    let out = client
        .prompt(&sid, prompt, |ev| match ev {
            ClientEvent::ApprovalRequested { .. } => {
                approvals += 1;
                EventAction::Approve(ApprovalDecision::AllowOnce)
            }
            ClientEvent::Barrier { phase } if phase == "hanging" => EventAction::Cancel,
            _ => EventAction::Continue,
        })
        .await
        .map_err(|e| e.to_string())?;
    println!("session={sid}");
    println!("stop={}", out.stop_reason);
    println!("approvals={approvals}");
    println!("text={}", out.text);
    Ok(())
}

fn cursor_bin() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("DESK_CURSOR_AGENT") {
        return Some(PathBuf::from(explicit));
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|dir| {
        let candidate = dir.join(format!("agent{}", std::env::consts::EXE_SUFFIX));
        candidate.is_file().then_some(candidate)
    })
}

fn image_fixture() -> Result<PathBuf, String> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("tests/fixtures/desk/z3p.png");
    if path.is_file() {
        return Ok(path);
    }
    Err(format!("missing image fixture {}", path.display()))
}

fn fake_cli_bin() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name(format!("desk-fake-cli{}", std::env::consts::EXE_SUFFIX));
        if sibling.exists() {
            return sibling;
        }
    }
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("target");
    path.push(if cfg!(debug_assertions) { "debug" } else { "release" });
    path.push(format!("desk-fake-cli{}", std::env::consts::EXE_SUFFIX));
    path
}
