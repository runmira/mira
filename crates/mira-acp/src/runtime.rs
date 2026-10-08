//! Provider-neutral lifecycle data. Native identifiers are scoped by the
//! owning engine instance and Mira session at the host boundary.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeCapabilities {
    pub steering: SteeringCapability,
    pub image_input: bool,
    pub live_model_switch: bool,
    pub live_mode_switch: bool,
    pub native_fork: bool,
    pub native_rollback: bool,
    pub native_snapshot: bool,
    pub cancellation: bool,
    pub stop_behavior: StopBehavior,
    pub asynchronous_questions: bool,
    pub background_work: bool,
    pub native_goals: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteeringCapability { #[default] Unavailable, Native, SafeBoundary }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopBehavior { #[default] CurrentTurn, Runtime }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeQuestion {
    pub id: String,
    pub question: String,
    pub header: Option<String>,
    pub options: Vec<String>,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub native_id: String,
    pub native_thread_id: String,
    pub native_turn_id: Option<String>,
    pub questions: Vec<RuntimeQuestion>,
    pub response_capability: ResponseCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseCapability {
    Message,
    LiveRpc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Goal,
    Task,
    Subagent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Pending,
    Running,
    Waiting,
    Paused,
    Completed,
    Failed,
    Cancelled,
}
impl WorkStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Waiting)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeWork {
    pub id: String,
    pub native_thread_id: Option<String>,
    pub kind: WorkKind,
    pub status: WorkStatus,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTurn {
    pub native_thread_id: String,
    pub native_turn_id: String,
    pub running: bool,
}

/// State scales with currently active work, not the number of completed
/// subagents. A terminal update removes its owner from the active index.
#[derive(Clone, Debug, Default)]
pub struct RuntimeActivity {
    turns: std::collections::HashSet<(String, String)>,
    work: std::collections::HashMap<String, RuntimeWork>,
}
impl RuntimeActivity {
    pub fn turn(&mut self, turn: &RuntimeTurn) {
        let key = (turn.native_thread_id.clone(), turn.native_turn_id.clone());
        if turn.running {
            self.turns.insert(key);
        } else {
            self.turns.remove(&key);
        }
    }
    pub fn work(&mut self, work: &RuntimeWork) {
        if work.status.is_active() {
            self.work.insert(work.id.clone(), work.clone());
        } else {
            self.work.remove(&work.id);
            if work.kind == WorkKind::Subagent {
                if let Some(thread) = work.native_thread_id.as_deref() {
                    self.turns.retain(|(id, _)| id != thread);
                }
            }
        }
    }
    pub fn end_turns(&mut self) -> Vec<RuntimeTurn> {
        self.turns
            .drain()
            .map(|(native_thread_id, native_turn_id)| RuntimeTurn {
                native_thread_id,
                native_turn_id,
                running: false,
            })
            .collect()
    }
    pub fn cancel_work(&mut self) -> Vec<RuntimeWork> {
        self.work.drain().map(|(_,mut work)| {work.status=WorkStatus::Cancelled;work}).collect()
    }
    pub fn has_pending_work(&self) -> bool {
        !self.turns.is_empty() || !self.work.is_empty()
    }
    pub fn work_snapshot(&self) -> Vec<RuntimeWork> {
        self.work.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_turn_end_does_not_release_a_running_background_task() {
        let mut a = RuntimeActivity::default();
        let mut turn = RuntimeTurn {
            native_thread_id: "t".into(),
            native_turn_id: "turn".into(),
            running: true,
        };
        a.turn(&turn);
        let mut task = RuntimeWork {
            id: "task".into(),
            native_thread_id: Some("t".into()),
            kind: WorkKind::Task,
            status: WorkStatus::Running,
            title: None,
        };
        a.work(&task);
        turn.running = false;
        a.turn(&turn);
        assert!(a.has_pending_work());
        task.status = WorkStatus::Completed;
        a.work(&task);
        assert!(!a.has_pending_work());
    }
    #[test]
    fn completed_work_does_not_accumulate_and_sessions_are_independent() {
        let mut a = RuntimeActivity::default();
        let b = RuntimeActivity::default();
        for i in 0..10000 {
            let mut w = RuntimeWork {
                id: i.to_string(),
                native_thread_id: None,
                kind: WorkKind::Subagent,
                status: WorkStatus::Running,
                title: None,
            };
            a.work(&w);
            assert!(a.has_pending_work());
            assert!(!b.has_pending_work());
            w.status = WorkStatus::Completed;
            a.work(&w);
        }
        assert!(a.work_snapshot().is_empty());
    }
}
