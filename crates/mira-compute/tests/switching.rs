//! Switching a session between the worktree and a remote environment.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use mira_compute::{EnvironmentManager, ExecRequest};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    std::fs::write(p.join(".gitignore"), ".cache/\n").unwrap();
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    std::fs::write(p.join("b.txt"), "b\n").unwrap();
    git(p, &["add", "-A"]);
    git(
        p,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    );
    dir
}

fn manager(project: &Path, patches: &Path) -> EnvironmentManager {
    let cfg: mira_config::ComputeConfig = serde_yaml::from_str(
        "environments:\n  dev:\n    backend: scratch\n    env: {GREETING: hello}\n    \
         setup: mkdir -p .cache && echo ran >> .cache/setup-count\n",
    )
    .unwrap();
    EnvironmentManager::new(project, patches, cfg)
}

fn progress() -> (mira_compute::env::Progress, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let l = log.clone();
    (Arc::new(move |m: String| l.lock().unwrap().push(m)), log)
}

#[tokio::test]
async fn round_trip_with_concurrent_local_edits() {
    let proj = project();
    let patches = tempfile::tempdir().unwrap();
    let p = proj.path();
    // Uncommitted local edit before switching: it must travel along.
    std::fs::write(p.join("b.txt"), "b edited locally\n").unwrap();

    let mgr = manager(p, patches.path());
    let slot = mgr.slot();
    let (prog, log) = progress();

    let r = mgr.switch("dev", prog.clone()).await.unwrap();
    assert_eq!((r.from.as_str(), r.to.as_str()), ("local", "dev"));
    assert!(r.model_note.contains("remote environment `dev`"));
    let backend = slot.get().expect("remote");
    assert_eq!(mgr.status().await.current, "dev");

    // Env vars, setup script, uploaded uncommitted edit.
    let out = backend
        .exec(
            ExecRequest::new("echo $GREETING; cat .cache/setup-count; cat b.txt"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out.stdout, "hello\nran\nb edited locally\n", "{out:?}");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .any(|m| m.contains("running setup")));

    // Work remotely (line 1) while the user edits locally (line 5).
    let a = String::from_utf8(backend.read_file("a.txt").await.unwrap()).unwrap();
    backend
        .write_file("a.txt", a.replace("one", "ONE").as_bytes())
        .await
        .unwrap();
    backend
        .write_file("new/remote.txt", b"from the sandbox\n")
        .await
        .unwrap();
    std::fs::write(p.join("a.txt"), "one\ntwo\nthree\nfour\nFIVE\n").unwrap();

    let r = mgr.switch("local", prog.clone()).await.unwrap();
    let pulled = r.pulled.expect("pulled");
    assert!(
        pulled.changed() && pulled.conflicts.is_empty(),
        "{pulled:?}"
    );
    assert!(pulled.patch_path.unwrap().exists());
    assert!(slot.get().is_none());
    assert_eq!(
        std::fs::read_to_string(p.join("a.txt")).unwrap(),
        "ONE\ntwo\nthree\nfour\nFIVE\n",
        "three-way merge kept both edits"
    );
    assert!(p.join("new/remote.txt").exists());
    assert!(r.model_note.contains("merged into the worktree"));
    assert_eq!(mgr.status().await.parked, ["dev"]);

    // Local work, then back to the parked environment: it's resynced,
    // setup doesn't rerun, ignored caches survive.
    std::fs::remove_file(p.join("b.txt")).unwrap();
    mgr.switch("dev", prog.clone()).await.unwrap();
    let backend = slot.get().unwrap();
    let out = backend
        .exec(
            ExecRequest::new("cat .cache/setup-count; ls; git status --short"),
            None,
        )
        .await
        .unwrap();
    assert!(out.stdout.starts_with("ran\n"), "setup ran once: {out:?}");
    assert!(
        !out.stdout.contains("b.txt"),
        "local deletion synced: {out:?}"
    );
    assert!(out.stdout.contains("remote.txt") || out.stdout.contains("new"));

    // Conflict: both sides change the same line.
    backend
        .write_file("a.txt", b"ONE\ntwo\nREMOTE\nfour\nFIVE\n")
        .await
        .unwrap();
    std::fs::write(p.join("a.txt"), "ONE\ntwo\nLOCAL\nfour\nFIVE\n").unwrap();
    let r = mgr.switch("local", prog.clone()).await.unwrap();
    let pulled = r.pulled.unwrap();
    assert_eq!(pulled.conflicts, ["a.txt"], "{pulled:?}");
    assert!(r.model_note.contains("merge conflicts"));
    let merged = std::fs::read_to_string(p.join("a.txt")).unwrap();
    assert!(merged.contains("<<<<<<<") && merged.contains("REMOTE") && merged.contains("LOCAL"));

    // Unknown targets fail without changing anything.
    assert!(mgr.switch("nope", prog.clone()).await.is_err());
    assert_eq!(mgr.status().await.current, "local");

    // Finish releases the parked scratch copy.
    assert!(mgr.finish().await.unwrap().is_none());
    assert!(mgr.status().await.parked.is_empty());
}

#[tokio::test]
async fn finish_saves_but_never_applies() {
    let proj = project();
    let patches = tempfile::tempdir().unwrap();
    let mgr = manager(proj.path(), patches.path());
    let (prog, _) = progress();
    mgr.switch("scratch", prog).await.unwrap();
    mgr.slot()
        .get()
        .unwrap()
        .write_file("a.txt", b"changed\n")
        .await
        .unwrap();
    let report = mgr.finish().await.unwrap().expect("report");
    assert!(report.changed());
    assert!(report.patch_path.unwrap().exists());
    assert_eq!(
        std::fs::read_to_string(proj.path().join("a.txt")).unwrap(),
        "one\ntwo\nthree\nfour\nfive\n"
    );
    assert!(mgr.slot().get().is_none());
}
