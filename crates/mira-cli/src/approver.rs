use async_trait::async_trait;
use mira_core::ToolCall;
use mira_harness::Approver;
use mira_policy::Decision;
use tokio::io::{AsyncBufReadExt, BufReader};

/// y/n prompt on stdin.
///
/// Intentionally simple: no fancy UI, no fuzzy answers. `y`/`yes` → allow,
/// anything else → deny. A richer TUI (ratatui) can substitute a modal.
pub struct TerminalApprover;

#[async_trait]
impl Approver for TerminalApprover {
    async fn approve(&self, call: &ToolCall, decision: Decision) -> bool {
        if decision == Decision::Allow {
            return true;
        }
        if decision == Decision::Deny {
            return false;
        }

        eprintln!("\n── approve tool call ──");
        eprintln!("  tool: {}", call.function.name);
        eprintln!("  args: {}", pretty(&call.function.arguments));
        eprint!("  allow? [y/N] ");

        let mut reader = BufReader::new(tokio::io::stdin());
        let mut line = String::new();
        if reader.read_line(&mut line).await.is_err() {
            return false;
        }
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

fn pretty(raw: &str) -> String {
    serde_json::from_str::<serde_json::Value>(raw)
        .and_then(|v| serde_json::to_string_pretty(&v))
        .unwrap_or_else(|_| raw.to_owned())
}
