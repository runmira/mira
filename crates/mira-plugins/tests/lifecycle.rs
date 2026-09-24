//! Marketplace + plugin lifecycle against local git repos (file://):
//! add, browse, detail, install (relative and git-subdir sources),
//! enabled components, disable, update, uninstall, remove.

use std::path::Path;
use std::process::Command;

use mira_plugins::PluginManager;

fn git(dir: &Path, args: &[&str]) {
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
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            msg,
        ],
    );
}

#[tokio::test]
async fn marketplace_and_plugin_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();

    // A separate repo holding a plugin in a subdirectory (git-subdir).
    let other = tmp.path().join("other");
    write(
        &other.join("plugins/remote-one/.claude-plugin/plugin.json"),
        r#"{"name": "remote-one", "version": "2.0.0", "description": "From another repo"}"#,
    );
    write(
        &other.join("plugins/remote-one/agents/helper.md"),
        "---\nname: helper\ndescription: helps\n---\nHelp.",
    );
    git(&other, &["init", "-q", "-b", "main"]);
    commit_all(&other, "init");

    // The marketplace repo.
    let market = tmp.path().join("market");
    write(
        &market.join(".claude-plugin/marketplace.json"),
        &format!(
            r#"{{
              "name": "test-market",
              "owner": {{"name": "Tester"}},
              "description": "For tests",
              "plugins": [
                {{"name": "tools", "source": "./plugins/tools", "description": "Local tools", "category": "development"}},
                {{"name": "remote-one", "source": {{"source": "git-subdir", "url": "file://{}", "path": "plugins/remote-one"}}}},
                {{"name": "from-npm", "source": {{"source": "npm", "package": "x"}}}}
              ]
            }}"#,
            other.display()
        ),
    );
    let tools = market.join("plugins/tools");
    write(
        &tools.join(".claude-plugin/plugin.json"),
        r#"{"name": "tools", "version": "1.0.0"}"#,
    );
    write(
        &tools.join("commands/greet.md"),
        "---\ndescription: Greet\n---\nHi $ARGUMENTS",
    );
    write(
        &tools.join("skills/notes/SKILL.md"),
        "---\nname: notes\ndescription: n\n---\nNotes.",
    );
    write(
        &tools.join("hooks/hooks.json"),
        r#"{"hooks": {"PreToolUse": []}}"#,
    );
    write(
        &tools.join(".mcp.json"),
        r#"{"mcpServers": {"db": {"command": "${CLAUDE_PLUGIN_ROOT}/bin/db", "args": ["--x"]}}}"#,
    );
    write(&tools.join("README.md"), "# Tools\nUseful.");
    git(&market, &["init", "-q", "-b", "main"]);
    commit_all(&market, "init");

    let mgr = PluginManager::new(tmp.path().join("home/plugins"));
    let name = mgr
        .add_marketplace(&format!("file://{}", market.display()))
        .await
        .unwrap();
    assert_eq!(name, "test-market");
    assert!(mgr
        .add_marketplace(&format!("file://{}", market.display()))
        .await
        .is_err());
    let markets = mgr.marketplaces().unwrap();
    assert_eq!(markets[0].plugin_count, 3);
    assert_eq!(markets[0].owner.as_deref(), Some("Tester"));

    let catalog = mgr.catalog().unwrap();
    assert_eq!(catalog.len(), 3);
    assert!(
        !catalog
            .iter()
            .find(|c| c.name == "from-npm")
            .unwrap()
            .installable
    );

    // Detail before installing: read from the marketplace clone.
    let d = mgr.detail("tools").unwrap();
    assert!(!d.entry.installed);
    let c = d.components.unwrap();
    assert_eq!(c.skills, ["notes"]);
    assert_eq!(c.hooks, ["PreToolUse"]);
    assert_eq!(d.command_names, ["greet"]);
    assert!(d.readme.unwrap().contains("Useful"));

    let t = mgr.install("tools@test-market").await.unwrap();
    assert_eq!(t.version, "1.0.0");
    assert!(t.path.join("commands/greet.md").is_file());
    let r = mgr.install("remote-one").await.unwrap();
    assert_eq!(r.version, "2.0.0");
    assert!(r.path.join("agents/helper.md").is_file());
    assert!(!r.path.join(".git").exists());
    assert!(mgr.install("from-npm").await.is_err());

    let enabled = mgr.enabled();
    assert_eq!(enabled.plugins.len(), 2);
    assert_eq!(enabled.skill_dirs().len(), 1);
    assert_eq!(enabled.agent_files().len(), 1);
    let (specs, problems) = enabled.mcp_specs();
    assert!(problems.is_empty());
    assert_eq!(specs[0].name, "plugin:tools:db");
    assert_eq!(specs[0].plugin_root.as_deref(), Some(t.path.as_path()));
    let cmds = mira_plugins::commands::load(None, None, &enabled.command_files());
    assert_eq!(cmds[0].name, "greet");
    assert_eq!(cmds[0].qualified.as_deref(), Some("tools:greet"));

    mgr.set_enabled("tools", false).await.unwrap();
    assert_eq!(mgr.enabled().plugins.len(), 1);
    assert!(
        !mgr.catalog()
            .unwrap()
            .iter()
            .find(|c| c.name == "tools")
            .unwrap()
            .enabled
    );

    // A new plugin lands in the marketplace; updating picks it up.
    write(
        &market.join("plugins/late/.claude-plugin/plugin.json"),
        r#"{"name": "late"}"#,
    );
    let mut m: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(market.join(".claude-plugin/marketplace.json")).unwrap(),
    )
    .unwrap();
    m["plugins"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"name": "late", "source": "./plugins/late"}));
    write(
        &market.join(".claude-plugin/marketplace.json"),
        &m.to_string(),
    );
    commit_all(&market, "add late");
    mgr.update_marketplace("test-market").await.unwrap();
    assert_eq!(mgr.catalog().unwrap().len(), 4);

    mgr.uninstall("remote-one").await.unwrap();
    assert!(!r.path.exists());
    assert_eq!(mgr.installed().unwrap().len(), 1);

    mgr.remove_marketplace("test-market").await.unwrap();
    assert!(mgr.installed().unwrap().is_empty());
    assert!(mgr.catalog().unwrap().is_empty());
    assert!(!t.path.exists());
}

/// Against Anthropic's real marketplace (network). Run with
/// `cargo test -p mira-plugins -- --ignored official`.
#[tokio::test]
#[ignore]
async fn official_marketplace() {
    let tmp = tempfile::tempdir().unwrap();
    let mgr = PluginManager::new(tmp.path().join("plugins"));
    let name = mgr
        .add_marketplace("anthropics/claude-plugins-official")
        .await
        .unwrap();
    let catalog = mgr.catalog().unwrap();
    eprintln!("{name}: {} plugins", catalog.len());
    assert!(catalog.len() > 50);
    let uninstallable: Vec<_> = catalog
        .iter()
        .filter(|c| !c.installable)
        .map(|c| &c.id)
        .collect();
    eprintln!("not installable: {uninstallable:?}");

    for id in [
        "commit-commands",
        "context7",
        "42crunch-api-security-testing",
        "agentforce-adlc",
    ] {
        let p = mgr
            .install(id)
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:#}"));
        eprintln!("installed {} {} at {}", p.id(), p.version, p.path.display());
    }
    let enabled = mgr.enabled();
    let cmds = mira_plugins::commands::load(None, None, &enabled.command_files());
    eprintln!(
        "commands: {:?}",
        cmds.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    assert!(cmds.iter().any(|c| c.name == "commit"));
    let (specs, problems) = enabled.mcp_specs();
    eprintln!(
        "mcp: {:?} {problems:?}",
        specs
            .iter()
            .map(|s| (&s.name, s.transport.summary()))
            .collect::<Vec<_>>()
    );
    assert!(specs.iter().any(|s| s.name == "plugin:context7:context7"));
    for (p, _, c) in mgr.installed().unwrap() {
        eprintln!(
            "{}: {} cmds, {} agents, skills {:?}, hooks {:?}, mcp {:?}, problems {:?}",
            p.id(),
            c.commands.len(),
            c.agents.len(),
            c.skills,
            c.hooks,
            c.mcp_server_names,
            c.problems
        );
    }
}
