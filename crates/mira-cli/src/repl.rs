use anyhow::Result;
use futures::StreamExt;
use mira_harness::{Goal, GoalStatus, HarnessEvent, Session};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run(session: Session) -> Result<()> {
    let mut stdout = io::stdout();
    let mut lines = BufReader::new(io::stdin()).lines();

    stdout
        .write_all(b"mira ready. type a message, ctrl-D to exit.\n")
        .await?;

    loop {
        stdout.write_all(b"\n> ").await?;
        stdout.flush().await?;

        let Some(line) = lines.next_line().await? else {
            stdout.write_all(b"\nbye.\n").await?;
            return Ok(());
        };
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        // Slash commands in the REPL are the small subset users hit in
        // headless / piped runs. TUI has the full palette.
        if let Some(cmd) = input.strip_prefix('/') {
            handle_repl_slash(cmd, &session, &mut stdout).await?;
            continue;
        }

        let mut events = session.send(input).await;
        while let Some(evt) = events.next().await {
            match evt {
                HarnessEvent::Token(t) => {
                    stdout.write_all(t.as_bytes()).await?;
                    stdout.flush().await?;
                }
                HarnessEvent::ToolStart(call) => {
                    let banner = format!(
                        "\n[tool] {}({})\n",
                        call.function.name, call.function.arguments
                    );
                    stdout.write_all(banner.as_bytes()).await?;
                    stdout.flush().await?;
                }
                HarnessEvent::ToolEnd(result) => {
                    let tag = if result.is_error { "err" } else { "ok" };
                    let banner = format!("[tool:{tag}] {}\n", oneline(&result.content));
                    stdout.write_all(banner.as_bytes()).await?;
                    stdout.flush().await?;
                }
                HarnessEvent::TurnComplete => {
                    // Between rounds within a single user turn — nothing to
                    // print, just a natural gap for the model to think.
                }
                HarnessEvent::Warning(w) => {
                    let msg = format!("\n[warn] {w}\n");
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::Usage { .. } => {
                    // repl doesn't render usage inline; totals are visible via
                    // `mira sessions ls`.
                }
                HarnessEvent::MemoryLearned { count } => {
                    let msg = format!("\n[memory] remembered {count} thing{}\n",
                        if count == 1 { "" } else { "s" });
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::Compacted { messages_removed } => {
                    let msg = format!(
                        "\n[context] compacted {messages_removed} earlier message{} into a summary\n",
                        if messages_removed == 1 { "" } else { "s" },
                    );
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::GoalSet { goal } => {
                    let msg = format!("\n[goal] set: {}\n", goal.condition);
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::GoalCleared => {
                    stdout.write_all(b"\n[goal] cleared\n").await?;
                }
                HarnessEvent::GoalProgress {
                    iteration,
                    max_iterations,
                    status,
                    reason,
                } => {
                    let word = goal_status_word(status);
                    let tail = reason
                        .as_ref()
                        .map(|r| format!(" · {r}"))
                        .unwrap_or_default();
                    let msg = format!(
                        "\n[goal] {iteration}/{max_iterations} · {word}{tail}\n"
                    );
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::GoalDone { status, reason } => {
                    let word = goal_status_word(status);
                    let tail = reason
                        .as_ref()
                        .map(|r| format!(" · {r}"))
                        .unwrap_or_default();
                    let msg = format!("\n[goal] {word}{tail}\n");
                    stdout.write_all(msg.as_bytes()).await?;
                }
                HarnessEvent::Done => {
                    stdout.write_all(b"\n").await?;
                    break;
                }
            }
        }
    }
}

fn oneline(s: &str) -> String {
    let s = s.trim();
    let first = s.lines().next().unwrap_or("");
    if s.contains('\n') {
        format!("{first} …")
    } else {
        first.to_owned()
    }
}

fn goal_status_word(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "still working",
        GoalStatus::Met => "met",
        GoalStatus::Impossible => "impossible",
        GoalStatus::NeedsUser => "needs you",
        GoalStatus::Cleared => "cleared",
        GoalStatus::Exhausted => "exhausted",
    }
}

/// REPL-side slash commands. Kept small: `/goal <cond>`,
/// `/goal status`, `/goal clear`, `/help`, `/quit`. Full palette
/// lives in the TUI.
async fn handle_repl_slash<W>(
    cmd: &str,
    session: &Session,
    stdout: &mut W,
) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let mut parts = cmd.splitn(2, ' ');
    let head = parts.next().unwrap_or("").trim();
    let rest = parts.next().unwrap_or("").trim();
    match head {
        "help" | "?" => {
            let msg = "commands: /goal <condition> · /goal status · /goal clear · /quit\n";
            stdout.write_all(msg.as_bytes()).await?;
        }
        "quit" | "q" => {
            stdout.write_all(b"bye.\n").await?;
            std::process::exit(0);
        }
        "goal" => {
            let sub = rest.trim();
            if sub.is_empty() || sub == "status" {
                match session.goal().await {
                    None => {
                        stdout
                            .write_all(b"no goal set. `/goal <condition>` to start.\n")
                            .await?;
                    }
                    Some(g) => {
                        let word = goal_status_word(g.status);
                        let msg = format!(
                            "goal · {} · {}/{} · {}\n",
                            word, g.iterations, g.max_iterations, g.condition
                        );
                        stdout.write_all(msg.as_bytes()).await?;
                        if let Some(r) = g.last_reason.as_ref() {
                            let m = format!("last note: {r}\n");
                            stdout.write_all(m.as_bytes()).await?;
                        }
                    }
                }
            } else if sub == "clear" {
                session.clear_goal().await;
                stdout.write_all(b"[goal] cleared\n").await?;
            } else {
                let g = Goal::new(sub);
                session.set_goal(g.clone()).await;
                let msg = format!("[goal] set: {}\n", g.condition);
                stdout.write_all(msg.as_bytes()).await?;
            }
        }
        other => {
            let msg = format!("unknown command `/{other}` — try /help\n");
            stdout.write_all(msg.as_bytes()).await?;
        }
    }
    stdout.flush().await?;
    Ok(())
}
