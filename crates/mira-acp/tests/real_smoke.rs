//! Smoke test against a real ACP agent, when one is available.
//!
//! Everything else in this crate is exercised against a shell script or an
//! in-memory pipe, which proves the protocol handling but says nothing about
//! whether the driver facts are right. This one spawns an actual agent.
//!
//! Skips cleanly when the binary is absent, so it is safe in CI.
use mira_acp::driver::{DriverConfig, PermissionMode};
use mira_acp::drivers::ClaudeCodeDriver;
use mira_acp::status::{probe, AgentState};

#[tokio::test]
async fn a_real_agent_completes_the_handshake() {
    let Ok(bin) = std::env::var("MIRA_ACP_TEST_BIN") else {
        eprintln!("skipping: set MIRA_ACP_TEST_BIN to a real adapter binary");
        return;
    };

    let cfg = DriverConfig {
        enabled: true,
        binary_path: Some(std::path::PathBuf::from(&bin)),
        ..Default::default()
    };
    let status = probe(&ClaudeCodeDriver, &cfg, PermissionMode::Ask).await;
    eprintln!("REAL PROBE: {status:#?}");

    assert_eq!(
        status.state,
        AgentState::Ready,
        "the real adapter did not initialize: {status:?}"
    );
    // This adapter authenticates through the `claude` CLI, so it advertises
    // no ACP auth methods at all. The version is what proves we read
    // `agentInfo` from a real response rather than a fixture.
    assert!(
        status.auth_method_ids.is_empty(),
        "expected no advertised auth methods, got {:?}",
        status.auth_method_ids
    );
    assert!(
        status.version.is_some(),
        "the real adapter should report a version in `agentInfo`"
    );
    assert!(!status.launch.contains("sk-"), "launch should be redacted");
}
