//! The hook end to end: a real panic leaves a redacted report, and nothing
//! at all when reports are off. Its own test binary, because it points
//! `HOME` at a temp dir and replaces the process-wide panic hook.

#[test]
fn panics_leave_a_redacted_report_only_when_turned_on() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", home.path());
    let crashes = home.path().join(".mira").join("crashes");

    // Off (the default): no hook, no file.
    mira_diagnostics::crash::install_panic_hook(&mira_config::MiraConfig::default());
    let _ = std::panic::catch_unwind(|| panic!("while off"));
    assert!(!crashes.exists());

    let cfg: mira_config::MiraConfig = serde_yaml::from_str(
        "diagnostics:\n  crash_reports: true\nproviders:\n  x:\n    api_key: custom-secret-value-1234\n",
    )
    .unwrap();
    mira_diagnostics::crash::install_panic_hook(&cfg);
    let _ = std::panic::catch_unwind(|| panic!("failed with key custom-secret-value-1234"));

    let pending = mira_diagnostics::crash::pending();
    assert_eq!(pending.len(), 1);
    let report = &pending[0].contents;
    assert!(
        report.contains("panic: failed with key «redacted»"),
        "{report}"
    );
    assert!(!report.contains("custom-secret-value-1234"));
    assert!(report.contains("backtrace:"));
}
