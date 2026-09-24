//! The manager against a real stdio server (tests/fixtures/fake_server.py):
//! connect, tools through a Registry, images, list_changed, roots,
//! resources, prompts, stderr logging, crash + reconnect, live edits,
//! disable, project approval, missing env vars and connect timeouts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::ToolCall;
use mira_mcp::{McpManager, McpOptions, Scope, ServerSpec, Status, Transport};
use mira_sandbox::Sandbox;
use mira_tools::{Registry, ToolContext};

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| {
        std::process::Command::new(p)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn fixture() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fake_server.py")
        .display()
        .to_string()
}

fn options(dir: &Path) -> McpOptions {
    McpOptions {
        connect_timeout: Duration::from_secs(10),
        tool_timeout: Duration::from_secs(10),
        max_output_chars: 10_000,
        credentials: dir.join("credentials.json"),
        state_file: dir.join("state.json"),
        log_dir: dir.join("logs"),
    }
}

fn spec(name: &str, scope: Scope, python: &str, env: &[(&str, &str)]) -> ServerSpec {
    ServerSpec {
        name: name.into(),
        scope,
        transport: Transport::Stdio {
            command: python.into(),
            args: vec![fixture()],
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cwd: None,
        },
        source: None,
        plugin_root: None,
    }
}

fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        kind: ToolCallKind::Function,
        function: ToolCallFunction {
            name: name.into(),
            arguments: args.to_string(),
        },
    }
}

async fn wait_for(mgr: &McpManager, name: &str, what: impl Fn(&Status) -> bool) -> Status {
    for _ in 0..200 {
        if let Some(s) = mgr.server(name) {
            if what(&s.status) {
                return s.status;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out; last: {:?}", mgr.server(name).map(|s| s.status));
}

#[tokio::test]
async fn full_lifecycle_against_a_stdio_server() {
    let Some(py) = python() else {
        eprintln!("skipping: no python");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mgr = McpManager::new(options(dir.path()));
    let mut events = mgr.subscribe();
    mgr.apply(
        vec![spec("fake", Scope::User, py, &[])],
        Some(project.clone()),
    );
    mgr.wait_settled(Duration::from_secs(10)).await;

    let view = mgr.server("fake").unwrap();
    assert_eq!(view.status, Status::Connected, "{view:?}");
    assert_eq!(view.server_version.as_deref(), Some("1.2.3"));
    assert_eq!(
        view.instructions.as_deref(),
        Some("Use echo to repeat things.")
    );
    assert!(view
        .tools
        .iter()
        .any(|t| t.name == "mcp__fake__echo" && t.read_only));
    assert_eq!(view.resources.len(), 1);
    assert_eq!(view.prompts[0].name, "review");
    assert!(events.try_recv().is_ok(), "status changes are broadcast");

    // stderr went to the log, not our terminal.
    let log = std::fs::read_to_string(view.log_path.as_ref().unwrap()).unwrap();
    assert!(log.contains("fake server starting"), "{log}");

    // Tools reach a registry live.
    let mut reg = Registry::new();
    reg.add_source(mgr.tool_source());
    let names: Vec<String> = reg.specs().into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"mcp__fake__echo".to_owned()), "{names:?}");
    assert!(names.contains(&"list_mcp_resources".to_owned()));
    let ctx = ToolContext::new(&project, Arc::new(Sandbox::new(&project)));

    let echo = reg.get("mcp__fake__echo").unwrap();
    assert_eq!(echo.action(), mira_tools::Action::Mcp);
    assert_eq!(
        echo.policy_target(&call("mcp__fake__echo", serde_json::json!({}))),
        "fake:echo"
    );
    let out = echo
        .invoke(
            &call("mcp__fake__echo", serde_json::json!({"text": "hi"})),
            &ctx,
        )
        .await
        .unwrap();
    assert_eq!(out.content, "hi");

    let pic = reg
        .get("mcp__fake__picture")
        .unwrap()
        .invoke(&call("mcp__fake__picture", serde_json::json!({})), &ctx)
        .await
        .unwrap();
    assert_eq!(pic.images.len(), 1);
    assert_eq!(pic.images[0].media_type, "image/png");

    // Roots: the server asks us, we answer with the project.
    let roots = reg
        .get("mcp__fake__roots")
        .unwrap()
        .invoke(&call("mcp__fake__roots", serde_json::json!({})), &ctx)
        .await
        .unwrap();
    assert!(roots.content.contains("file://"), "{}", roots.content);
    assert!(roots.content.contains("project"), "{}", roots.content);

    // Resources and prompts.
    let read = reg
        .get("read_mcp_resource")
        .unwrap()
        .invoke(
            &call(
                "read_mcp_resource",
                serde_json::json!({"server": "fake", "uri": "mem://notes"}),
            ),
            &ctx,
        )
        .await
        .unwrap();
    assert!(read.content.contains("remember the milk"));
    let mut args = BTreeMap::new();
    args.insert("file".to_owned(), "main.rs".to_owned());
    assert_eq!(
        mgr.get_prompt("fake", "review", args).await.unwrap(),
        "Please review main.rs."
    );

    // list_changed: a new tool shows up without reconnecting.
    mgr.call_tool("fake", "add_tool", None).await.unwrap();
    for _ in 0..100 {
        if reg.get("mcp__fake__extra").is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(reg.get("mcp__fake__extra").is_some(), "tool added live");

    // The process dies: the manager notices and brings it back.
    let _ = mgr.call_tool("fake", "crash", None).await;
    wait_for(&mgr, "fake", |s| matches!(s, Status::Failed { .. })).await;
    assert!(reg.get("mcp__fake__echo").is_none(), "no tools while down");
    wait_for(&mgr, "fake", |s| *s == Status::Connected).await;
    assert!(reg.get("mcp__fake__echo").is_some());

    // Disable and re-enable (remembered in state.json).
    mgr.set_enabled("fake", false).unwrap();
    assert_eq!(mgr.server("fake").unwrap().status, Status::Disabled);
    assert!(reg.get("mcp__fake__echo").is_none());
    mgr.set_enabled("fake", true).unwrap();
    wait_for(&mgr, "fake", |s| *s == Status::Connected).await;

    // Removing it from the config disconnects it.
    mgr.apply(vec![], Some(project.clone()));
    assert!(mgr.server("fake").is_none());
    assert!(reg.specs().is_empty());
    mgr.shutdown();
}

#[tokio::test]
async fn project_servers_wait_for_approval() {
    let Some(py) = python() else { return };
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().to_path_buf();
    let mgr = McpManager::new(options(dir.path()));
    mgr.apply(
        vec![spec("repo", Scope::Project, py, &[])],
        Some(project.clone()),
    );
    assert_eq!(mgr.server("repo").unwrap().status, Status::NeedsApproval);
    mgr.set_approved("repo", true).unwrap();
    wait_for(&mgr, "repo", |s| *s == Status::Connected).await;

    // The repo changes what the server runs: approval is asked again.
    mgr.apply(
        vec![spec("repo", Scope::Project, py, &[("CHANGED", "1")])],
        Some(project.clone()),
    );
    assert_eq!(mgr.server("repo").unwrap().status, Status::NeedsApproval);
    mgr.set_approved("repo", false).unwrap();
    assert_eq!(mgr.server("repo").unwrap().status, Status::Rejected);
    mgr.shutdown();
}

#[tokio::test]
async fn clear_failures_for_bad_definitions() {
    let Some(py) = python() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut opts = options(dir.path());
    opts.connect_timeout = Duration::from_secs(1);
    let mgr = McpManager::new(opts);
    let hang = ServerSpec {
        name: "hang".into(),
        scope: Scope::User,
        transport: Transport::Stdio {
            command: py.into(),
            args: vec!["-c".into(), "import time; time.sleep(30)".into()],
            env: Default::default(),
            cwd: None,
        },
        source: None,
        plugin_root: None,
    };
    let missing_var = ServerSpec {
        name: "needs-token".into(),
        transport: Transport::Stdio {
            command: py.into(),
            args: vec![fixture()],
            env: [("TOKEN".to_owned(), "${MIRA_TEST_SURELY_UNSET}".to_owned())].into(),
            cwd: None,
        },
        ..hang.clone()
    };
    let no_binary = ServerSpec {
        name: "nope".into(),
        transport: Transport::Stdio {
            command: "/definitely/not/a/binary".into(),
            args: vec![],
            env: Default::default(),
            cwd: None,
        },
        ..hang.clone()
    };
    let started = std::time::Instant::now();
    mgr.apply(vec![hang, missing_var, no_binary], None);
    mgr.wait_settled(Duration::from_secs(10)).await;
    // All three in parallel: well under 3 × the timeout.
    assert!(started.elapsed() < Duration::from_secs(3));
    let msg = |n: &str| match mgr.server(n).unwrap().status {
        Status::Failed { message } => message,
        other => panic!("{n}: {other:?}"),
    };
    assert!(msg("hang").contains("MCP_TIMEOUT"), "{}", msg("hang"));
    assert!(
        msg("needs-token").contains("MIRA_TEST_SURELY_UNSET"),
        "{}",
        msg("needs-token")
    );
    assert!(msg("nope").contains("couldn't start"), "{}", msg("nope"));
    let _: PathBuf = dir.path().into();
    mgr.shutdown();
}
