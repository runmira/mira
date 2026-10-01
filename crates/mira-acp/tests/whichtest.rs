use mira_acp::driver::{AcpDriver, DriverConfig, PermissionMode};
use std::path::PathBuf;

use mira_acp::drivers::ClaudeCodeDriver;
use mira_acp::native::PermissionGate;
use std::sync::Arc;

fn bare_path() {
    let bare = std::ffi::OsString::from("/usr/bin:/bin:/usr/sbin:/sbin");
    unsafe { std::env::set_var("PATH", &bare) };
}

/// The reported failure: `could not start claude: No such file or directory`.
/// The agent was validated as present, then the spawn failed — because the
/// child gets a cleared environment whose PATH could not find `claude`.
#[tokio::test]
async fn a_native_agent_actually_starts_from_a_gui_launch() {
    bare_path();
    if mira_acp::which::resolve("claude").is_none() {
        return;
    }

    let launch = mira_acp::native::launch(PathBuf::from("claude"), PermissionMode::Ask, None, None);

    let gate: PermissionGate =
        Arc::new(|_p| Box::pin(async { mira_acp::native::PermissionDecision::from(true) }));
    let res = mira_acp::native::NativeAgent::start(&launch, gate, None).await;
    match &res {
        Ok(a) => {
            eprintln!("SPAWN ok, stderr={:?}", a.stderr_tail().await);
        }
        Err(e) => panic!("spawn failed: {e}"),
    }
    // The CLI only emits `init` once it has been spoken to, so the real proof
    // that the pipe works is a full turn: write a message, read a `result`.
    // The account may be rate-limited, and that still produces a `result`,
    // so this is not gated on the agent being willing to work.
    let agent = res.unwrap();
    let mut events = agent
        .events
        .lock()
        .expect("events")
        .take()
        .expect("events rx");
    let mut ends = agent
        .turn_end
        .lock()
        .expect("turn end")
        .take()
        .expect("turn end rx");

    agent
        .prompt("Reply with exactly: PONG")
        .await
        .expect("write prompt");

    let got = tokio::time::timeout(std::time::Duration::from_secs(90), async {
        let mut saw_text = false;
        loop {
            tokio::select! {
                Some(e) = events.recv() => {
                    if let mira_acp::events::MiraEvent::AssistantText { text, .. } = &e.event {
                        eprintln!("TURN text={text}");
                        saw_text = true;
                    }
                }
                Some(end) = ends.recv() => {
                    eprintln!("TURN end stop_reason={} error={}", end.stop_reason, end.is_error);
                    return Some((saw_text, end));
                }
                else => return None,
            }
        }
    })
    .await;

    let Some((_, _end)) = got.expect("the agent never ended its turn") else {
        panic!("the event stream closed without the turn ending");
    };
    eprintln!("TURN session_id={:?}", agent.session_id().await);
    assert!(
        agent.session_id().await.is_some(),
        "a session id must be reported"
    );
    agent.shutdown().await;
}

/// The same at the ACP layer, which had the identical latent bug.
#[test]
fn the_acp_spawn_resolves_its_program_too() {
    bare_path();
    if mira_acp::which::resolve("sh").is_none() {
        return;
    }
    let p = mira_acp::which::resolve_for_spawn(std::path::Path::new("sh"));
    assert!(p.is_absolute(), "a bare name must not be handed to exec");
}

/// The resolved program must be the one the check validated.
#[test]
fn check_and_spawn_agree_on_the_binary() {
    bare_path();
    let d = ClaudeCodeDriver;
    // Needs Claude Code installed; CI machines don't have it.
    if mira_acp::which::resolve(d.binary_names()[0]).is_none() {
        eprintln!("skipped: {} is not installed", d.binary_names()[0]);
        return;
    }
    let cfg = DriverConfig::default();
    let program = PathBuf::from(d.binary_names()[0]);
    let native_bin = mira_acp::process::native_program(&d, &cfg, &program);
    let spawn = mira_acp::which::resolve_for_spawn(&native_bin);
    eprintln!("AGREE spawn={:?}", spawn);
    assert!(spawn.is_absolute());
    assert!(
        spawn.exists(),
        "the validated binary must exist at spawn time"
    );
    assert!(!spawn.to_string_lossy().contains("acp"));
}

/// The reported failure: the composer kept loading after the agent had
/// finished, and the user had to stop it by hand.
///
/// The cause was client-side — the `acp_turn_end` handler recorded the stop
/// reason but never cleared `busy` — which no amount of server work can fix.
/// This asserts the server half of the contract: a turn that ends produces
/// exactly one `TurnEnd` carrying the agent's own stop reason, and an agent
/// that dies mid-turn still produces one rather than nothing.
#[tokio::test]
async fn a_turn_always_produces_exactly_one_stop_reason() {
    bare_path();
    if mira_acp::which::resolve("claude").is_none() {
        return;
    }
    let launch = mira_acp::native::launch(PathBuf::from("claude"), PermissionMode::Ask, None, None);
    let gate: PermissionGate =
        Arc::new(|_p| Box::pin(async { mira_acp::native::PermissionDecision::from(true) }));
    let agent = mira_acp::native::NativeAgent::start(&launch, gate, None)
        .await
        .expect("spawn");
    let mut events = agent.events.lock().expect("ev").take().expect("rx");
    let mut ends = agent.turn_end.lock().expect("te").take().expect("rx");

    agent
        .prompt("Reply with exactly: PONG")
        .await
        .expect("prompt");

    let first = tokio::time::timeout(std::time::Duration::from_secs(90), async {
        loop {
            tokio::select! {
                _ = events.recv() => {}
                Some(end) = ends.recv() => return Some(end),
                else => return None,
            }
        }
    })
    .await
    .expect("turn ended")
    .expect("the stream closed without a stop reason");

    eprintln!(
        "LIFECYCLE first stop_reason={} error={}",
        first.stop_reason, first.is_error
    );
    assert!(
        !first.stop_reason.is_empty(),
        "a stop reason must be reported"
    );

    // No second end for the same turn: the client would double-count it.
    let extra = tokio::time::timeout(std::time::Duration::from_secs(3), ends.recv()).await;
    assert!(extra.is_err(), "a turn must end exactly once");
    agent.shutdown().await;
}

/// An agent that exits without a `result` must still end the turn, or the
/// composer waits on a process that is no longer there.
#[tokio::test]
async fn an_agent_that_dies_mid_turn_still_closes_the_turn() {
    bare_path();
    // A stand-in that announces a session and then exits without a result,
    // which is what a crash looks like on the wire.
    let dir = tempfile::tempdir().expect("tmp");
    let script = dir.path().join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\n\
         echo '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"x1\"}'\n\
         exit 1\n",
    )
    .expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let launch = mira_acp::native::launch(script, PermissionMode::Ask, None, None);
    let gate: PermissionGate =
        Arc::new(|_p| Box::pin(async { mira_acp::native::PermissionDecision::from(true) }));
    let agent = mira_acp::native::NativeAgent::start(&launch, gate, None)
        .await
        .expect("spawn");
    let mut ends = agent.turn_end.lock().expect("te").take().expect("rx");

    // No `result` is ever emitted; the only way the client learns the turn is
    // over is the stream closing. The server's pump turns that into a stop.
    // Either a `result` arrives (the script's exit is a normal end here) or
    // the channel closes — the server covers the closed case. What must never
    // happen is the channels staying open with nothing sent.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(10), ends.recv()).await;
    agent.shutdown().await;
    assert!(
        outcome.is_ok(),
        "a crashed agent must end the turn or close the channel, not hang"
    );
}
