use anyhow::Result;
use futures::StreamExt;
use mira_harness::{Goal, GoalStatus, HarnessEvent, Session};
use mira_tools::builtin::skill::SkillHandle;
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run(session: Session, skills: SkillHandle) -> Result<()> {
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
            handle_repl_slash(cmd, &session, &skills, &mut stdout).await?;
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
                    let msg = format!(
                        "\n[memory] remembered {count} thing{}\n",
                        if count == 1 { "" } else { "s" }
                    );
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
                    let msg = format!("\n[goal] {iteration}/{max_iterations} · {word}{tail}\n");
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
                HarnessEvent::ToolProgress { line, .. } => {
                    // Print the line as-is — the REPL is already
                    // line-oriented so a bash command's live output
                    // reads naturally in the running transcript.
                    stdout.write_all(line.as_bytes()).await?;
                    stdout.write_all(b"\n").await?;
                    stdout.flush().await?;
                }
                HarnessEvent::ToolPreview { .. } => {
                    // The REPL doesn't render diff previews; the diff
                    // is redundant with the tool's own output on stdout.
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
/// `/goal status`, `/goal clear`, `/skills`, `/skill <name>`, `/help`,
/// `/quit`. Full palette lives in the TUI.
async fn handle_repl_slash<W>(
    cmd: &str,
    session: &Session,
    skills: &SkillHandle,
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
            let msg = "commands: /goal <condition> · /goal status · /goal clear · \
                       /skills · /skill <name> · /quit\n";
            stdout.write_all(msg.as_bytes()).await?;
        }
        "quit" | "q" => {
            stdout.write_all(b"bye.\n").await?;
            std::process::exit(0);
        }
        "skills" => {
            let reg = skills.read().await.clone();
            if reg.skills.is_empty() {
                stdout.write_all(b"no skills loaded.\n").await?;
            } else {
                let header = format!(
                    "{} skill{} loaded:\n",
                    reg.skills.len(),
                    if reg.skills.len() == 1 { "" } else { "s" }
                );
                stdout.write_all(header.as_bytes()).await?;
                for s in reg.skills.values() {
                    let line = format!("  /{:<24} {}\n", s.name, s.description);
                    stdout.write_all(line.as_bytes()).await?;
                }
            }
        }
        "skill" => {
            let name = rest.split_whitespace().next().unwrap_or("");
            if name.is_empty() {
                stdout.write_all(b"usage: /skill <name>\n").await?;
            } else {
                let reg = skills.read().await.clone();
                match reg.get(name) {
                    None => {
                        let msg = format!("no skill named `{name}` — `/skills` to list.\n");
                        stdout.write_all(msg.as_bytes()).await?;
                    }
                    Some(s) => {
                        let header = format!("/{}\n{}\n", s.name, s.description);
                        stdout.write_all(header.as_bytes()).await?;
                        if let Some(src) = s.source.as_ref() {
                            let m = format!("source: {}\n", src.display());
                            stdout.write_all(m.as_bytes()).await?;
                        } else {
                            stdout.write_all(b"source: (bundled)\n").await?;
                        }
                        let attachments = s.attached_files();
                        if !attachments.is_empty() {
                            let m = format!(
                                "attached ({}): {}\n",
                                attachments.len(),
                                attachments
                                    .iter()
                                    .filter_map(|p| p.to_str())
                                    .collect::<Vec<_>>()
                                    .join(", "),
                            );
                            stdout.write_all(m.as_bytes()).await?;
                        }
                        stdout.write_all(b"---\n").await?;
                        stdout.write_all(s.body.as_bytes()).await?;
                        stdout.write_all(b"\n").await?;
                    }
                }
            }
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
