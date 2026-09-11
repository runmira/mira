use anyhow::Result;
use futures::StreamExt;
use mira_harness::{HarnessEvent, Session};
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
