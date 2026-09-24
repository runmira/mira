//! Remote environments from the terminal: `--sandbox <env>` at startup
//! and `/remote-env` mid-session both drive the session's
//! [`EnvironmentManager`]. This module is the CLI-side glue: which
//! environment to start in, and how switches and the end-of-session
//! patch are reported.

use std::path::Path;

use mira_compute::workspace::PullReport;
use mira_compute::SwitchReport;
use mira_config::ComputeConfig;

/// Environment to start in: `--sandbox` wins over `compute.default`.
/// `None` (or `local`) means this machine.
pub fn startup_target(flag: Option<&str>, cfg: &ComputeConfig) -> Option<String> {
    flag.map(str::to_owned)
        .or_else(|| cfg.default.clone())
        .filter(|t| t != mira_compute::env::LOCAL)
}

/// Human-readable lines describing a finished switch.
pub fn describe_switch(r: &SwitchReport) -> Vec<String> {
    if r.from == r.to {
        return vec![format!("already in `{}`", r.to)];
    }
    let mut lines = vec![format!("environment: {} → {}", r.from, r.to)];
    if let Some(p) = &r.pulled {
        if p.changed() {
            lines.push(format!(
                "merged changes from `{}` into the worktree:",
                r.from
            ));
            lines.extend(p.stat.lines().map(|l| format!("  {l}")));
            if !p.conflicts.is_empty() {
                lines.push(format!("conflicts to resolve: {}", p.conflicts.join(", ")));
            }
            if let Some(path) = &p.patch_path {
                lines.push(format!("(patch kept at {})", path.display()));
            }
        } else {
            lines.push(format!("`{}` had no changes", r.from));
        }
    }
    lines
}

/// Print what happened to the active environment's changes at exit.
pub fn print_finish(result: mira_compute::Result<Option<PullReport>>, project: &Path) {
    match result {
        Ok(Some(p)) if p.changed() => {
            let path = p.patch_path.expect("saved patch has a path");
            eprintln!(
                "\nremote environment: changes saved to {}\n{}\n\nApply them with:\n  git -C {} apply {}",
                path.display(),
                p.stat.trim_end(),
                project.display(),
                path.display()
            );
        }
        Ok(Some(_)) => eprintln!("remote environment: no changes."),
        Ok(None) => {}
        Err(e) => eprintln!("warning: collecting the remote environment's changes failed: {e}"),
    }
}
