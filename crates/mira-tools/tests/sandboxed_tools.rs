//! Routed tools in a sandboxed session: everything lands in the backend's
//! workspace and nothing touches the "local" checkout.

use std::sync::Arc;

use mira_compute::{ComputeBackend, LocalBackend};
use mira_core::message::{ToolCallFunction, ToolCallKind};
use mira_core::ToolCall;
use mira_tools::builtin::{self, bash, edit, glob, grep, read, write};
use mira_tools::{Registry, Tool, ToolContext};
use serde_json::{json, Value};

fn call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        kind: ToolCallKind::Function,
        function: ToolCallFunction {
            name: name.into(),
            arguments: args.to_string(),
        },
    }
}

async fn run(tool: &dyn Tool, ctx: &ToolContext, args: Value) -> String {
    let name = tool.spec().name;
    let r = tool.invoke(&call(&name, args), ctx).await.unwrap();
    assert!(!r.is_error, "{name}: {}", r.content);
    r.content
}

#[tokio::test]
async fn routed_tools_work_in_the_backend_workspace() {
    let local = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    std::fs::create_dir(remote.path().join("src")).unwrap();
    std::fs::write(remote.path().join("src/lib.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    // The sandbox workspace is a git repo (as after `workspace::upload`).
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(remote.path())
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success();
    assert!(ok);

    let backend: Arc<dyn ComputeBackend> = Arc::new(LocalBackend::new(remote.path()));
    let ctx = ToolContext::new(
        local.path(),
        Arc::new(mira_sandbox::Sandbox::new(local.path())),
    )
    .with_compute(backend);

    // Paths may be relative, local-absolute, or sandbox-absolute.
    let out = run(&read::ReadFile, &ctx, json!({"path": "src/lib.rs"})).await;
    assert!(out.contains("1\tfn a() {}"), "{out}");
    let local_abs = local.path().join("src/lib.rs");
    let out = run(
        &read::ReadFile,
        &ctx,
        json!({"path": local_abs, "start_line": 2}),
    )
    .await;
    assert!(out.contains("fn b") && !out.contains("fn a"), "{out}");

    run(
        &edit::EditFile,
        &ctx,
        json!({"path": "src/lib.rs", "old_string": "fn b() {}", "new_string": "fn b() { todo!() }"}),
    )
    .await;
    run(
        &write::WriteFile,
        &ctx,
        json!({"path": "notes/todo.md", "content": "hi\n"}),
    )
    .await;
    assert!(std::fs::read_to_string(remote.path().join("src/lib.rs"))
        .unwrap()
        .contains("todo!()"));
    assert!(remote.path().join("notes/todo.md").exists());

    let out = run(&bash::Bash, &ctx, json!({"command": "ls notes && pwd"})).await;
    assert!(out.contains("exit=0") && out.contains("todo.md"), "{out}");

    let out = run(&grep::Grep, &ctx, json!({"pattern": "todo!"})).await;
    assert!(out.contains("src/lib.rs"), "{out}");
    let out = run(&glob::Glob, &ctx, json!({"pattern": "**/*.md"})).await;
    assert_eq!(out.trim(), "notes/todo.md");

    // Escapes are refused; the local checkout was never touched.
    let r = read::ReadFile
        .invoke(&call("read_file", json!({"path": "../x"})), &ctx)
        .await;
    assert!(r.is_err());
    assert_eq!(std::fs::read_dir(local.path()).unwrap().count(), 0);
}

#[test]
fn remote_environments_see_only_remote_capable_tools() {
    let mut reg = Registry::new();
    builtin::register_core(&mut reg);
    builtin::register_memory(&mut reg);
    let removed = reg.local_only();
    let kept: Vec<String> = reg.remote_specs().into_iter().map(|s| s.name).collect();
    for t in [
        "bash",
        "read_file",
        "write_file",
        "edit_file",
        "grep",
        "glob",
        "task_list",
        "web_fetch",
    ] {
        assert!(kept.contains(&t.to_owned()), "{t} should stay: {kept:?}");
    }
    for t in ["apply_patch", "git_status", "find_symbol", "rustfmt"] {
        assert!(
            removed.contains(&t.to_owned()),
            "{t} should be withheld: {removed:?}"
        );
    }
}
