//! Full-screen ratatui frontend for the Mira harness.
//!
//! Module layout — three layers with one job each:
//!
//! - [`state`] owns *what is happening* (entries, composer, viewport,
//!   derived-layout cache).
//! - [`render`] + [`components`] own *how things look* (blocks → lines)
//!   and *where they go* ([`render::layout`]).
//! - [`event_loop`] + [`input`] own *what happens when the world moves*
//!   (terminal events, harness events, approvals).
//!
//! Slash commands (`/help`, `/mode`, `/model`, `/clear`, `/quit`, …)
//! live here at the crate face since they're the CLI surface. `Ctrl+C`
//! drops the current agent stream — the harness's spawned loop task
//! exits when its send channel closes.

pub mod approver;
mod components;
pub(crate) mod event_loop;
mod input;
mod markdown;
pub(crate) mod render;
mod state;
mod theme;

pub use approver::TuiApprover;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use mira_harness::{Goal, GoalStatus, Session, SessionStore};
use mira_policy::{Mode, Policy};
use mira_tools::builtin::skill::SkillHandle;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, Mutex};

use event_loop::{current_cost_usd, event_loop, format_dollars_short};

/// Everything the TUI needs beyond what `Session` already owns.
pub struct TuiConfig {
    pub model: String,
    pub mode: Mode,
    /// Handle to the shared policy so `/mode` can update it live.
    pub policy: Arc<Mutex<Policy>>,
    /// Receiver paired with the [`TuiApprover`] handed to `Session`.
    pub approval_rx: mpsc::UnboundedReceiver<approver::ApprovalRequest>,
    /// Repo root — used to resolve relative paths in edit/write diff previews.
    pub cwd: std::path::PathBuf,
    /// Loaded skill registry — the same handle the `Skill` tool consults.
    /// Backs `/skills` (list) and `/skill <name>` (detail) so the user can
    /// inspect the roster without leaving the TUI.
    pub skills: SkillHandle,
    /// Session persistence store, when the run has one. Backs
    /// `/sessions` (list recent in cwd) so the user can find and
    /// re-launch prior conversations without leaving the TUI.
    /// `None` when persistence is disabled (`--no-persist`).
    pub store: Option<Arc<dyn SessionStore>>,
    /// Live provider's model catalog — populated in the background at
    /// boot via `ChatProvider::list_models`. Read on every `/model `
    /// palette open so autocomplete has the real per-provider ids
    /// (`google/gemini-2.5-flash`, `sonnet-4-6`, …). Empty when the
    /// fetch is in flight or the provider doesn't expose a catalog;
    /// the palette handles both by silently showing no completions.
    pub models: Arc<tokio::sync::RwLock<Vec<String>>>,
}

/// Built-in slash commands the palette suggests. Order is display order.
pub(crate) const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "list commands"),
    (
        "/mode",
        "switch permission mode (plan|manual|auto|edit|yolo)",
    ),
    ("/model", "switch model for this session"),
    ("/goal", "set / inspect / clear the standing goal"),
    ("/skills", "list loaded skills"),
    ("/skill", "show one skill in detail"),
    ("/permissions", "list session policy · add \"Rule(...)\""),
    ("/undo", "revert the last file write in this session"),
    ("/save", "export the transcript as markdown"),
    ("/sessions", "list recent sessions in this folder"),
    ("/resume", "show shell command to resume a session"),
    (
        "/budget",
        "cap this session's spend (e.g. /budget $2 · /budget off)",
    ),
    ("/cost", "print token & dollar breakdown for this session"),
    (
        "/theme",
        "swap palette (`/theme` · `/theme <name>` · `/theme reload` · `/theme save`)",
    ),
    ("/clear", "clear the visible transcript"),
    ("/quit", "exit the TUI"),
];

pub async fn run(session: Session, cfg: TuiConfig) -> Result<()> {
    let mut terminal = enter()?;
    // Track the terminating state's mouse-capture flag so `leave`
    // can pair a matching Disable with any Enable the event loop
    // toggled on. Starts `false` because text selection works by
    // default; Alt+M enables capture for scroll-wheel driving.
    let mut mouse_capture_on = false;
    let outcome = event_loop(&mut terminal, session, cfg, &mut mouse_capture_on).await;
    leave(&mut terminal, mouse_capture_on)?;
    outcome
}

// ---- terminal lifecycle ----

fn enter() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    enable_raw_mode()?;
    let mut out = std::io::stdout();
    // Text selection works by default. Users who want native
    // text selection can toggle with Alt+M — most terminals also honour
    // Option/Shift+drag as a "bypass capture" selection modifier.
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn leave(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mouse_capture: bool,
) -> Result<()> {
    disable_raw_mode()?;
    if mouse_capture {
        let _ = execute!(terminal.backend_mut(), crossterm::event::DisableMouseCapture);
    }
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;
    Ok(())
}

// ---- slash commands ----

async fn run_slash(
    cmd: &str,
    state: &mut state::TuiState,
    session: &Session,
    cfg: &mut TuiConfig,
) -> Option<String> {
    let mut parts = cmd.trim().splitn(2, ' ');
    let head = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();

    match head {
        "/quit" | "/q" => state.should_quit = true,

        "/clear" => {
            // clear_entries only touches the transcript + scroll — mode,
            // model, goal, usage totals, and policy stay put so /clear
            // reads as a "focus reset", not a session teardown. The info
            // line below reminds the user what's preserved.
            state.clear_entries();
            let mut kept = format!(
                "cleared · model {} · mode {}",
                state.model,
                state.mode.as_str()
            );
            if let Some(g) = state.goal.as_ref() {
                kept.push_str(&format!(" · goal {}/{}", g.iterations, g.max_iterations));
            }
            state.push_info(kept);
            state.flash = Some("cleared".into());
        }

        "/help" | "/?" => {
            state.push_info(
                "commands: /mode <plan|manual|auto|edit|yolo> · /model <id> · \
                 /goal <cond> · /goal status · /goal clear · /skills · \
                 /skill <name> · /budget <$X|off> · \
                 /permissions [add \"Rule(...)\"] · \
                 /undo [N] · /save [path] · /clear · /quit  ·  \
                 keys: @ file · / cmd · ctrl+r search · \
                 ctrl+y copy last reply · ctrl+e expand last tool · \
                 ctrl+w kill word · ctrl+↑↓ jump turns · ctrl+x drop paste · \
                 ctrl+z undo write · ctrl+p retry last turn · 1-9 toggle tools",
            );
            // Also enumerate the mounted skill slashes — they change
            // per project, so hard-coding them in the line above would
            // rot. `/skills` still shows the full detail view.
            let reg = cfg.skills.read().await.clone();
            let mut aliases: Vec<String> = reg
                .skills
                .values()
                .filter_map(|s| s.slash.as_deref().map(|a| format!("/{a}")))
                .filter(|s| !is_reserved_slash(s))
                .collect();
            aliases.sort();
            aliases.dedup();
            if !aliases.is_empty() {
                state.push_info(format!("skill slashes: {}", aliases.join(" · ")));
            }
        }

        "/goal" => run_goal_slash(rest, state, session).await,

        "/skills" => run_skills_slash(state, &cfg.skills).await,

        "/skill" => run_skill_slash(rest, state, &cfg.skills).await,

        "/save" => run_save_slash(rest, state, &cfg.cwd),

        "/sessions" => run_sessions_slash(state, cfg).await,

        "/resume" => run_resume_slash(rest, state, cfg).await,

        "/undo" => run_undo_slash(rest, state, session).await,

        "/permissions" | "/perms" => run_permissions_slash(rest, state, cfg).await,

        "/mode" => match parse_mode(rest) {
            Some(m) => {
                state.mode = m;
                cfg.policy.lock().await.set_mode(m);
                state.flash = Some(format!("mode → {}", m.as_str()));
            }
            None => state.push_warning(format!("unknown mode `{rest}`")),
        },

        "/model" => {
            if rest.is_empty() {
                state.push_warning("usage: /model <id>".into());
            } else {
                state.model = rest.to_owned();
                session.set_model(rest.to_owned()).await;
                state.flash = Some(format!("model → {rest}"));
            }
        }

        "/budget" => match parse_budget(rest) {
            Ok(None) => {
                state.budget_usd = None;
                state.flash = Some("budget cleared".into());
            }
            Ok(Some(cap)) => {
                state.budget_usd = Some(cap);
                state.flash = Some(format!("budget → ${cap:.2}"));
                if let Some(spent) = current_cost_usd(state) {
                    if spent >= cap {
                        state.push_warning(format!(
                            "already at {} — next send blocked until you raise or clear the cap",
                            format_dollars_short(spent)
                        ));
                    }
                }
            }
            Err(msg) => state.push_warning(msg),
        },

        "/cost" => {
            let u = state.usage;
            if u.is_zero() {
                state.push_info("no usage reported yet".to_string());
            } else {
                let mut line = format!(
                    "cost · ↑{} ↓{} (cached {}) · {} rounds",
                    u.prompt_tokens, u.completion_tokens, u.cached_input_tokens, u.rounds
                );
                if let Some(spent) = current_cost_usd(state) {
                    line.push_str(&format!(" · {}", format_dollars_short(spent)));
                    if let Some(cap) = state.budget_usd {
                        let left = (cap - spent).max(0.0);
                        line.push_str(&format!(" / ${cap:.2} cap · ${left:.3} left"));
                    }
                } else if state.budget_usd.is_some() {
                    line.push_str(" · (model unpriced — budget won't trip)");
                }
                state.push_info(line);
            }
        }

        "/theme" => run_theme_slash(rest, state),

        // Fall through to the skill registry: any skill whose
        // frontmatter declares `slash: X` (default `X = skill.name`)
        // mounts as `/X`. Reserved commands above always win.
        other => {
            let alias = other.trim_start_matches('/');
            if let Some(skill_name) = find_skill_by_slash(alias, &cfg.skills).await {
                state.flash = Some(format!("skill → {skill_name}"));
                let arg_line = if rest.is_empty() {
                    String::new()
                } else {
                    format!("\n\nInvocation arg: {rest}")
                };
                return Some(format!("Please invoke the `{skill_name}` skill.{arg_line}"));
            }
            state.push_warning(format!("unknown command `{other}` — try /help"));
        }
    }
    None
}

/// Parse the argument to `/budget`. Accepts `off`, `clear`, `none` for
/// disable; otherwise strips a leading `$` and reads an f64.
fn parse_budget(rest: &str) -> Result<Option<f64>, String> {
    let t = rest.trim();
    if t.is_empty() {
        return Err("usage: /budget $X · /budget off".into());
    }
    if matches!(t.to_ascii_lowercase().as_str(), "off" | "clear" | "none") {
        return Ok(None);
    }
    let raw = t.trim_start_matches('$').trim();
    let n: f64 = raw
        .parse()
        .map_err(|_| format!("can't parse `{t}` as a dollar amount"))?;
    if !n.is_finite() || n <= 0.0 {
        return Err(format!("budget must be positive · got {t}"));
    }
    Ok(Some(n))
}

/// `/theme` sub-dispatch.
///
/// Shape:
/// - `/theme`            — list bundled presets + point at the yaml.
/// - `/theme <name>`     — apply a bundled preset (in-memory).
/// - `/theme reload`     — re-read `~/.mira/theme.yaml`.
/// - `/theme save`       — write the current palette to yaml (persists).
fn run_theme_slash(rest: &str, state: &mut state::TuiState) {
    let rest = rest.trim();
    if rest.is_empty() {
        state.push_info(format!(
            "theme file: {}",
            crate::tui::theme::theme_path().display()
        ));
        state.push_info("presets:".to_string());
        for (name, _, desc) in crate::tui::theme::PRESETS {
            state.push_info(format!("  {name:<10} — {desc}"));
        }
        state.push_info("usage: /theme <name> · /theme reload · /theme save".to_string());
        return;
    }
    match rest {
        "reload" => match crate::tui::theme::reload() {
            Ok(path) => {
                state.flash = Some("theme reloaded".into());
                state.push_info(format!("theme ← {}", path.display()));
            }
            Err(e) => state.push_warning(format!("reload failed: {e}")),
        },
        "save" => match crate::tui::theme::save_current_to_disk() {
            Ok(path) => {
                state.flash = Some("theme saved".into());
                state.push_info(format!("theme → {}", path.display()));
            }
            Err(e) => state.push_warning(format!("save failed: {e}")),
        },
        name => match crate::tui::theme::preset(name) {
            Some((t, desc)) => {
                crate::tui::theme::set(t);
                state.flash = Some(format!("theme → {name}"));
                state.push_info(format!("theme · {name} — {desc}"));
            }
            None => state.push_warning(format!(
                "unknown theme `{name}` — try /theme to list presets"
            )),
        },
    }
}

/// True when `head` (with the leading `/`) is one of the built-in slash
/// commands. Built-ins always win over a skill alias — a skill named
/// `mode.md` can't shadow `/mode`.
pub(crate) fn is_reserved_slash(head: &str) -> bool {
    SLASH_COMMANDS.iter().any(|(name, _)| *name == head) || matches!(head, "/q" | "/?" | "/perms")
}

/// Look up a skill by its slash alias. Returns the underlying skill
/// name so callers can synthesize an invocation regardless of whether
/// the alias matches the skill's own name (`/verify` → `verify`) or
/// renames it (`/review` → `code-review`).
async fn find_skill_by_slash(
    slash: &str,
    skills: &mira_tools::builtin::skill::SkillHandle,
) -> Option<String> {
    let reg = skills.read().await.clone();
    reg.skills
        .values()
        .find(|s| s.slash.as_deref() == Some(slash))
        .map(|s| s.name.clone())
}

/// Dispatch for `/goal ...` — the sub-verb decides.
///
/// Shape:
/// - `/goal <condition>`     — set (or replace) the standing goal
/// - `/goal status`          — print the current goal
/// - `/goal clear`           — drop the goal
async fn run_goal_slash(rest: &str, state: &mut state::TuiState, session: &Session) {
    let rest = rest.trim();
    if rest.is_empty() || rest == "status" {
        // Snapshot to a local so the immutable borrow on `state.goal`
        // ends before we call the `&mut` push_info helpers.
        let snapshot = state.goal.clone();
        match snapshot {
            None => state.push_info(
                "no goal set. `/goal <condition>` to start an autonomous run.".to_string(),
            ),
            Some(g) => {
                let status_word = match g.status {
                    GoalStatus::Active => "active",
                    GoalStatus::Met => "met",
                    GoalStatus::Impossible => "impossible",
                    GoalStatus::NeedsUser => "needs you",
                    GoalStatus::Cleared => "cleared",
                    GoalStatus::Exhausted => "exhausted",
                };
                state.push_info(format!(
                    "goal · {status_word} · {}/{} · {}",
                    g.iterations, g.max_iterations, g.condition
                ));
                if let Some(r) = g.last_reason.as_ref() {
                    state.push_info(format!("last note: {r}"));
                }
            }
        }
        return;
    }
    if rest == "clear" {
        session.clear_goal().await;
        state.goal = None;
        state.push_info("[goal] cleared".to_string());
        state.flash = Some("goal cleared".into());
        return;
    }
    // Otherwise treat `rest` as the goal condition — set it.
    let goal = Goal::new(rest);
    session.set_goal(goal.clone()).await;
    state.goal = Some(goal.clone());
    state.push_info(format!("[goal] set: {}", goal.condition));
    state.flash = Some("goal set".into());
}

/// `/skills` — one line per loaded skill (bundled + user + project
/// merged, same view the composer palette in `mira serve` sees). Fires
/// as info entries so scroll-back keeps them.
async fn run_skills_slash(
    state: &mut state::TuiState,
    skills: &SkillHandle,
) {
    let reg = skills.read().await.clone();
    if reg.skills.is_empty() {
        state.push_info(
            "no skills loaded. drop a SKILL.md into ~/.mira/skills/<name>/ to add one.".to_string(),
        );
        return;
    }
    state.push_info(format!(
        "{} skill{} loaded:",
        reg.skills.len(),
        if reg.skills.len() == 1 { "" } else { "s" }
    ));
    for s in reg.skills.values() {
        state.push_info(format!("  /{:<24} {}", s.name, s.description));
    }
}

/// `/skill <name>` — description + source + attachment list + body.
/// Emitted line-by-line so the terminal transcript stays scroll-back
/// searchable rather than being one giant blob.
async fn run_skill_slash(rest: &str, state: &mut state::TuiState, skills: &SkillHandle) {
    let name = rest.split_whitespace().next().unwrap_or("").trim();
    if name.is_empty() {
        state.push_warning("usage: /skill <name>".into());
        return;
    }
    let reg = skills.read().await.clone();
    let Some(s) = reg.get(name).cloned() else {
        state.push_warning(format!("no skill named `{name}` — `/skills` to list."));
        return;
    };
    state.push_info(format!("/{} — {}", s.name, s.description));
    match s.source.as_ref() {
        Some(p) => state.push_info(format!("source: {}", p.display())),
        None => state.push_info("source: (bundled)".to_string()),
    }
    let attachments = s.attached_files();
    if !attachments.is_empty() {
        state.push_info(format!(
            "attached ({}): {}",
            attachments.len(),
            attachments
                .iter()
                .filter_map(|p| p.to_str())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    state.push_info("---".to_string());
    for line in s.body.lines() {
        state.push_info(line.to_string());
    }
}

/// `/undo [N]` — revert the last N file writes made in this session.
/// Defaults to 1. No-op with a clean flash message when there's
/// nothing on the guard's stack (or no guard at all).
async fn run_undo_slash(rest: &str, state: &mut state::TuiState, session: &Session) {
    let n: usize = rest.trim().parse().unwrap_or(1);
    let Some(guard) = session.file_guard() else {
        state.push_warning("undo isn't wired for this session (no FileGuard)".into());
        return;
    };
    match guard.undo(n).await {
        Ok(applied) => {
            if applied.is_empty() {
                state.push_info("nothing to undo");
                return;
            }
            let paths: Vec<&str> = applied.iter().map(|a| a.path.as_str()).collect();
            state.push_info(format!(
                "[undo] reverted {} write{}: {}",
                applied.len(),
                if applied.len() == 1 { "" } else { "s" },
                paths.join(", ")
            ));
            state.flash = Some("undone".into());
        }
        Err(e) => state.push_warning(format!("undo failed: {e}")),
    }
}

/// `/permissions` — inspect or extend the session policy.
///
/// Shape:
/// - `/permissions`                    → list current allow rules
/// - `/permissions add "Rule(...)"`    → session-scoped add
/// - `/permissions add Bash(cargo test:*)` → also accepted (quotes optional)
async fn run_permissions_slash(rest: &str, state: &mut state::TuiState, cfg: &TuiConfig) {
    let rest = rest.trim();
    if rest.is_empty() || rest == "ls" || rest == "list" {
        let policy = cfg.policy.lock().await;
        let n = policy.allow_count();
        state.push_info(format!(
            "policy · mode {} · {} allow rule{}",
            policy.mode().as_str(),
            n,
            if n == 1 { "" } else { "s" }
        ));
        state.push_info(
            "session-scoped rules aren't persisted; run `mira config edit` to save.".to_string(),
        );
        return;
    }
    if let Some(rest) = rest.strip_prefix("add ").map(str::trim) {
        // Strip a single pair of surrounding quotes if present.
        let rule = rest
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .unwrap_or(rest);
        match cfg.policy.lock().await.add_allow_rule(rule) {
            Ok(_) => {
                state.push_info(format!("[policy] session-allow: {rule}"));
                state.flash = Some("rule added".into());
            }
            Err(e) => state.push_warning(format!("bad rule `{rule}`: {e}")),
        }
        return;
    }
    state.push_warning("usage: /permissions  ·  /permissions add \"Rule(pattern)\"".into());
}

/// `/save [path]` — dump the visible transcript to a markdown file.
/// Default path is `<cwd>/.mira/transcript-<epoch>.md`; a caller-supplied
/// path is used verbatim.
fn run_save_slash(rest: &str, state: &mut state::TuiState, cwd: &std::path::Path) {
    let path: PathBuf = if rest.is_empty() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        cwd.join(".mira").join(format!("transcript-{ts}.md"))
    } else {
        PathBuf::from(rest)
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut out = String::new();
    for e in state.entries() {
        match e {
            state::LogEntry::User(s) => {
                out.push_str("**you:** ");
                out.push_str(s);
                out.push_str("\n\n");
            }
            state::LogEntry::TurnEnd { elapsed_ms, .. } => {
                out.push_str(&format!(
                    "_· {}_\n\n",
                    crate::tui::components::status::turn_end_label(*elapsed_ms)
                ));
            }
            state::LogEntry::Assistant(s) => {
                out.push_str(s);
                out.push_str("\n\n");
            }
            state::LogEntry::ToolCall { name, args, .. } => {
                out.push_str(&format!("_tool call:_ `{name}({args})`\n\n"));
            }
            state::LogEntry::ToolResult {
                ok, snippet, full, ..
            } => {
                let mark = if *ok { "✓" } else { "✗" };
                let body = if full.is_empty() { snippet } else { full };
                out.push_str(&format!("_tool result {mark}:_\n```\n{body}\n```\n\n"));
            }
            state::LogEntry::Warning(s) => {
                out.push_str(&format!("> ⚠ {s}\n\n"));
            }
            state::LogEntry::Info(s) => {
                out.push_str(&format!("_{s}_\n\n"));
            }
            state::LogEntry::Welcome { model, cwd, tip, .. } => {
                out.push_str(&format!(
                    "_mira · {model} · {cwd}_\n\n> {tip}\n\n"
                ));
            }
        }
    }
    match std::fs::write(&path, out) {
        Ok(_) => {
            state.push_info(format!("saved transcript → {}", path.display()));
            state.flash = Some("saved".into());
        }
        Err(e) => state.push_warning(format!("save failed: {e}")),
    }
}

/// `/sessions` — list up to 15 recent sessions in the current cwd
/// with a title + relative-time chip. Read-only: switching to a
/// different session requires re-launching mira (see `/resume`).
async fn run_sessions_slash(state: &mut state::TuiState, cfg: &TuiConfig) {
    let Some(store) = cfg.store.as_ref() else {
        state.push_warning("session persistence is off (`--no-persist`?) — nothing to list".into());
        return;
    };
    match store.list_recent(&cfg.cwd, 15).await {
        Err(e) => state.push_warning(format!("couldn't list sessions: {e}")),
        Ok(list) if list.is_empty() => {
            state.push_info(format!(
                "no saved sessions in {} — try coming back after a chat lands.",
                cfg.cwd.display()
            ));
        }
        Ok(list) => {
            state.push_info(format!("recent sessions in {}:", cfg.cwd.display()));
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            for rec in list {
                let id_short = short_session_id(rec.id.as_str());
                let title = rec
                    .title
                    .clone()
                    .or_else(|| first_user_message_from_record(&rec))
                    .unwrap_or_else(|| "(no messages yet)".into());
                let title = title.trim().replace('\n', " ");
                let title = if title.chars().count() > 60 {
                    let head: String = title.chars().take(60).collect();
                    format!("{head}…")
                } else {
                    title
                };
                let age = fmt_relative(now.saturating_sub(rec.updated_at));
                state.push_info(format!("  {id_short}  ·  {age:>8}  ·  {title}"));
            }
            state.push_info(
                "resume: run `mira --resume <id>` (or `mira --pick`) from a new shell.".to_string(),
            );
        }
    }
}

/// `/resume` — the TUI can't hot-swap sessions safely (Session owns a
/// bunch of channels wired into the harness loop task). Instead, we
/// print the exact shell command that resumes the target session, so
/// the user can `Ctrl+C` out and paste it. `<id>` optional — omitted
/// resumes the most recent.
async fn run_resume_slash(rest: &str, state: &mut state::TuiState, cfg: &TuiConfig) {
    let target = rest.trim();
    let cmd = if target.is_empty() {
        "mira --resume ''".to_string()
    } else {
        format!("mira --resume {target}")
    };
    state.push_info(
        "resume can't hot-swap sessions mid-run — the harness owns the current one. \
         quit and run this in your shell:"
            .to_string(),
    );
    state.push_info(format!("  $ {cmd}"));
    if !target.is_empty() {
        state.push_info(
            "(or `mira --pick` for an interactive picker over recent sessions)".to_string(),
        );
    }
    // Also: if we know the session store, verify the id exists so users
    // don't quit only to hit "not found".
    if !target.is_empty() {
        if let Some(store) = cfg.store.as_ref() {
            if let Err(e) = store.load(&mira_core::SessionId::from(target)).await {
                state.push_warning(format!("(note: session lookup failed: {e})"));
            }
        }
    }
}

fn short_session_id(id: &str) -> String {
    // Session ids are usually uuid-like; the first 8 chars are enough
    // to disambiguate within a cwd. Fall back to the whole thing for
    // short custom ids.
    if id.chars().count() > 12 {
        id.chars().take(8).collect::<String>()
    } else {
        id.to_owned()
    }
}

fn first_user_message_from_record(rec: &mira_harness::SessionRecord) -> Option<String> {
    rec.messages
        .iter()
        .find(|m| m.role == mira_core::Role::User)
        .and_then(|m| m.content.clone())
}

/// Coarse "5m ago", "2d ago". Only used in `/sessions` so precision
/// doesn't matter past the right unit.
fn fmt_relative(secs: u64) -> String {
    if secs < 60 {
        "just now".to_owned()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn parse_mode(s: &str) -> Option<Mode> {
    match s {
        "plan" => Some(Mode::Plan),
        "manual" => Some(Mode::Manual),
        "auto" => Some(Mode::Auto),
        "edit" => Some(Mode::Edit),
        "yolo" => Some(Mode::Yolo),
        _ => None,
    }
}
