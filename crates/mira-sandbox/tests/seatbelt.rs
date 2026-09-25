//! Runs real commands under sandbox-exec (macOS only; CI covers it).

#![cfg(target_os = "macos")]

use mira_sandbox::{Sandbox, SandboxBackend, SandboxProfile};

async fn sh(profile: SandboxProfile, script: &str) -> mira_sandbox::CommandOutput {
    let cwd = profile.repo_root.clone();
    Sandbox::with_profile(profile)
        .run("sh", &["-c".to_owned(), script.to_owned()], &cwd)
        .await
        .unwrap()
}

#[tokio::test]
async fn seatbelt_confines_commands() {
    if Sandbox::new("/").backend() != SandboxBackend::Seatbelt {
        eprintln!("skipping: sandbox-exec not available");
        return;
    }
    let repo = tempfile::tempdir().unwrap();
    let repo_path = repo.path().canonicalize().unwrap();
    std::fs::create_dir_all(repo_path.join(".git/hooks")).unwrap();
    let profile = SandboxProfile::new(&repo_path);

    // The repository is writable.
    let out = sh(profile.clone(), "echo hi > made && cat made").await;
    assert_eq!(out.stdout.trim(), "hi", "{out:?}");

    // Git hooks aren't, even inside the repository.
    let out = sh(profile.clone(), "echo x > .git/hooks/pre-commit").await;
    assert!(!out.success(), "{out:?}");
    assert!(!repo_path.join(".git/hooks/pre-commit").exists());

    // Nor is the rest of HOME.
    let home = dirs::home_dir().unwrap().canonicalize().unwrap();
    let outside = home.join(format!("mira-seatbelt-{}", std::process::id()));
    let out = sh(
        profile.clone(),
        &format!("echo x > '{}'", outside.display()),
    )
    .await;
    let escaped = outside.exists();
    let _ = std::fs::remove_file(&outside);
    assert!(!escaped, "{out:?}");

    // No network unless the profile allows it.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!("/bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/{port}' && echo connected");
    let out = sh(profile.clone(), &script).await;
    assert!(!out.stdout.contains("connected"), "{out:?}");
    let out = sh(profile.network(true), &script).await;
    assert!(out.stdout.contains("connected"), "{out:?}");
}
