//! Durable asynchronous questions and their answer outbox. A resolution and
//! its user message commit in the same atomic file replacement, before any
//! provider call. Repeated answers return the original receipt.
use mira_acp::runtime::{ResponseCapability, RuntimeRequest};
use mira_tools::prompt::{AskUserOption, AskUserProposal, AskUserQuestion, AskUserResponse};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    Pending,
    Queued,
    Dispatching,
    Delivered,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestRecord {
    pub id: String,
    pub instance: String,
    pub driver: String,
    pub request: RuntimeRequest,
    pub delivery: Delivery,
    pub response: Option<AskUserResponse>,
    pub message: Option<String>,
}
impl RequestRecord {
    pub fn proposal(&self) -> AskUserProposal {
        AskUserProposal {
            questions: self
                .request
                .questions
                .iter()
                .map(|q| AskUserQuestion {
                    question: q.question.clone(),
                    header: q.header.clone(),
                    multi_select: false,
                    options: q
                        .options
                        .iter()
                        .map(|label| AskUserOption {
                            label: label.clone(),
                            description: None,
                            recommended: false,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

pub struct RequestStore {
    path: Option<PathBuf>,
    records: Mutex<BTreeMap<String, RequestRecord>>,
    load_error: Option<String>,
    closed: std::sync::atomic::AtomicBool,
}
impl RequestStore {
    pub async fn open(path: Option<PathBuf>) -> Self {
        let loaded = match &path {
            Some(p) => match tokio::fs::read(p).await {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
                Err(e) => Err(e.to_string()),
            },
            None => Ok(BTreeMap::new()),
        };
        let (records, load_error) = match loaded {
            Ok(m) => (m, None),
            Err(e) => (BTreeMap::new(), Some(e)),
        };
        Self {
            path,
            records: Mutex::new(records),
            load_error,
            closed: std::sync::atomic::AtomicBool::new(false),
        }
    }
    pub async fn close(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        // Drain any write already in flight before deleting persistence.
        let _drain = self.records.lock().await;
    }
    pub fn reopen(&self) {
        self.closed
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
    async fn commit(&self, records: &BTreeMap<String, RequestRecord>) -> Result<(), String> {
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("this session's runtime store is closed".into());
        }
        if let Some(e) = &self.load_error {
            return Err(format!("could not load runtime requests: {e}"));
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let path = path.clone();
        let bytes = serde_json::to_vec(records).map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("request store has no parent"))?;
            std::fs::create_dir_all(parent)?;
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut f = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)?;
                f.write_all(&bytes)?;
                f.sync_all()?;
                std::fs::rename(&temp, &path)?;
                std::fs::File::open(parent)?.sync_all()?;
                Ok(())
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(temp);
            }
            result
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
    }
    pub fn is_persistent(&self) -> bool {
        self.path.is_some() && self.load_error.is_none()
    }

    pub async fn insert(
        &self,
        instance: &str,
        driver: &str,
        request: RuntimeRequest,
    ) -> Result<RequestRecord, String> {
        if request.response_capability != ResponseCapability::Message
            || request.questions.is_empty()
            || request.native_thread_id.is_empty()
        {
            return Err("invalid asynchronous request".into());
        }
        if request.native_id.is_empty()
            || request.native_id.len() > 1024
            || request.native_thread_id.len() > 1024
            || request.questions.len() > 32
            || serde_json::to_vec(&request)
                .map_err(|e| e.to_string())?
                .len()
                > 65536
        {
            return Err("asynchronous request exceeds the supported payload limit".into());
        }
        let mut records = self.records.lock().await;
        if let Some(r) = records.values().find(|r| {
            r.instance == instance
                && r.request.native_thread_id == request.native_thread_id
                && r.request.native_id == request.native_id
        }) {
            return Ok(r.clone());
        }
        if records.len() >= 4096 {
            return Err("this session has reached its durable request limit".into());
        }
        let r = RequestRecord {
            id: format!("async-{}", uuid::Uuid::new_v4()),
            instance: instance.into(),
            driver: driver.into(),
            request,
            delivery: Delivery::Pending,
            response: None,
            message: None,
        };
        let mut next = records.clone();
        next.insert(r.id.clone(), r.clone());
        self.commit(&next).await?;
        *records = next;
        Ok(r)
    }
    pub async fn resolve(
        &self,
        id: &str,
        response: AskUserResponse,
    ) -> Result<Option<RequestRecord>, String> {
        let mut records = self.records.lock().await;
        let Some(mut r) = records.get(id).cloned() else {
            return Ok(None);
        };
        if r.delivery != Delivery::Pending {
            return Ok(Some(r));
        }
        if response.cancelled {
            r.delivery = Delivery::Cancelled;
        } else {
            if response.answers.len() != r.request.questions.len() {
                return Err("answer every required question before submitting".into());
            }
            let mut lines = Vec::new();
            for (q, a) in r.request.questions.iter().zip(&response.answers) {
                let custom = a.custom.as_deref().unwrap_or("").trim();
                if q.required && a.picked.is_empty() && custom.is_empty() {
                    return Err(format!("answer required: {}", q.question));
                }
                if a.picked.iter().any(|p| !q.options.contains(p)) {
                    return Err("answer contains an unknown option".into());
                }
                lines.push(format!(
                    "{}\n{}",
                    q.question,
                    if custom.is_empty() {
                        a.picked.join(", ")
                    } else {
                        custom.to_owned()
                    }
                ));
            }
            r.message = Some(format!(
                "Answers to your questions:\n\n{}",
                lines.join("\n\n")
            ));
            r.delivery = Delivery::Queued;
        }
        if serde_json::to_vec(&response)
            .map_err(|e| e.to_string())?
            .len()
            > 65536
        {
            return Err("answers exceed the supported payload limit".into());
        }
        r.response = Some(response);
        let mut next = records.clone();
        next.insert(id.into(), r.clone());
        self.commit(&next).await?;
        *records = next;
        Ok(Some(r))
    }
    pub async fn snapshot(&self) -> Vec<RequestRecord> {
        self.records.lock().await.values().cloned().collect()
    }
    pub async fn transition(&self, id: &str, from: Delivery, to: Delivery) -> Result<bool, String> {
        let mut records = self.records.lock().await;
        let Some(r) = records.get(id) else {
            return Ok(false);
        };
        if r.delivery != from {
            return Ok(false);
        }
        let mut next = records.clone();
        next.get_mut(id).unwrap().delivery = to;
        self.commit(&next).await?;
        *records = next;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mira_acp::runtime::RuntimeQuestion;
    use mira_tools::prompt::AskUserAnswer;
    fn request() -> RuntimeRequest {
        RuntimeRequest {
            native_id: "item".into(),
            native_thread_id: "thread".into(),
            native_turn_id: Some("turn".into()),
            questions: vec![RuntimeQuestion {
                id: "q".into(),
                question: "Choose".into(),
                header: None,
                options: vec!["A".into()],
                required: true,
            }],
            response_capability: ResponseCapability::Message,
        }
    }
    fn answer(s: &str) -> AskUserResponse {
        AskUserResponse {
            cancelled: false,
            answers: vec![AskUserAnswer {
                picked: vec![],
                custom: Some(s.into()),
            }],
        }
    }
    #[tokio::test]
    async fn restart_and_duplicate_answers_keep_one_original_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let store = RequestStore::open(Some(path.clone())).await;
        let r = store
            .insert("codex-work", "codex", request())
            .await
            .unwrap();
        assert_eq!(
            store
                .insert("codex-work", "codex", request())
                .await
                .unwrap()
                .id,
            r.id
        );
        assert_ne!(
            store
                .insert("codex-personal", "codex", request())
                .await
                .unwrap()
                .id,
            r.id
        );
        let store = RequestStore::open(Some(path.clone())).await;
        assert_eq!(store.snapshot().await[0].delivery, Delivery::Pending);
        let resolved = store
            .resolve(&r.id, answer("first"))
            .await
            .unwrap()
            .unwrap();
        let duplicate = store
            .resolve(&r.id, answer("second"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.message, duplicate.message);
        let store = RequestStore::open(Some(path)).await;
        let persisted = store
            .snapshot()
            .await
            .into_iter()
            .find(|x| x.id == r.id)
            .unwrap();
        assert_eq!(persisted.delivery, Delivery::Queued);
        assert!(persisted.message.unwrap().contains("first"));
    }
    #[tokio::test]
    async fn validation_and_corrupt_storage_fail_closed() {
        let store = RequestStore::open(None).await;
        let r = store.insert("a", "codex", request()).await.unwrap();
        assert!(store.resolve(&r.id, answer("  ")).await.is_err());
        assert_eq!(store.snapshot().await[0].delivery, Delivery::Pending);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        tokio::fs::write(&path, b"broken").await.unwrap();
        let store = RequestStore::open(Some(path.clone())).await;
        assert!(store.insert("a", "codex", request()).await.is_err());
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
    }
    #[tokio::test]
    async fn concurrent_resolution_commits_one_outbox_message() {
        let store = RequestStore::open(None).await;
        let r = store.insert("a", "codex", request()).await.unwrap();
        let (a, b) = tokio::join!(
            store.resolve(&r.id, answer("A")),
            store.resolve(&r.id, answer("B"))
        );
        assert_eq!(a.unwrap().unwrap().message, b.unwrap().unwrap().message);
        assert!(store
            .transition(&r.id, Delivery::Queued, Delivery::Dispatching)
            .await
            .unwrap());
        assert!(!store
            .transition(&r.id, Delivery::Queued, Delivery::Dispatching)
            .await
            .unwrap());
    }
}

/// One worker only while this session has queued answers. No polling timer
/// per parked session: native lifecycle changes wake the worker through Notify.
pub fn wake_outbox(
    state: &crate::state::AppState,
    slot: &std::sync::Arc<crate::slot::SessionSlot>,
) {
    use std::sync::atomic::Ordering;
    if slot.engine.outbox_running.swap(true, Ordering::SeqCst) {
        slot.engine.activity_changed.notify_one();
        return;
    }
    let state = state.clone();
    let weak = std::sync::Arc::downgrade(slot);
    tokio::spawn(async move {
        let Some(slot) = weak.upgrade() else {
            return;
        };
        loop {
            // Register before inspecting state so an end cannot be lost.
            let changed = slot.engine.activity_changed.notified();
            let _dispatch = slot.engine.dispatch_lock.lock().await;
            let instance = slot
                .acp_launch
                .lock()
                .await
                .as_ref()
                .map(|p| p.instance.clone());
            let queued = slot
                .runtime_requests
                .snapshot()
                .await
                .into_iter()
                .find(|r| r.delivery == Delivery::Queued && Some(&r.instance) == instance.as_ref());
            let Some(record) = queued else {
                // Under the dispatch lock, release ownership then recheck in
                // the caller on the next resolution/selection/ready event.
                state.runtime_retries.remove(&slot.id.to_string()).await;
                slot.engine.outbox_running.store(false, Ordering::SeqCst);
                break;
            };
            if state
                .slot(&slot.id)
                .await
                .as_ref()
                .is_none_or(|s| !std::sync::Arc::ptr_eq(s, &slot))
            {
                slot.engine.outbox_running.store(false, Ordering::SeqCst);
                break;
            }
            let handle = match crate::session_engine::ensure_agent(&state, &slot).await {
                Ok(h) => h,
                Err(e) => {
                    slot.engine.outbox_running.store(false, Ordering::SeqCst);
                    if e.contains("capacity reached") {
                        state.runtime_retries.park(&slot).await;
                    } else {
                        let _ = slot.events_tx.send(crate::protocol::ServerMsg::Warning {
                            text: format!("your answers are saved and queued: {e}"),
                        });
                    }
                    break;
                }
            };
            state.runtime_retries.remove(&slot.id.to_string()).await;
            let Some(agent) = handle.agent().await else {
                slot.engine.outbox_running.store(false, Ordering::SeqCst);
                break;
            };
            if agent.session_id().await.as_deref() != Some(&record.request.native_thread_id) {
                let _ = slot.events_tx.send(crate::protocol::ServerMsg::Warning { text: "your saved answers belong to an earlier native conversation and were not sent to this one".into() });
                slot.engine.outbox_running.store(false, Ordering::SeqCst);
                break;
            }
            let running = slot.is_running().await;
            let can_steer = match &agent {
                mira_acp::native::AgentHandle::AppServer(a) => a.has_active_turn().await,
                _ => false,
            };
            if running && !can_steer {
                drop(_dispatch);
                changed.await;
                continue;
            }
            if !slot
                .runtime_requests
                .transition(&record.id, Delivery::Queued, Delivery::Dispatching)
                .await
                .unwrap_or(false)
            {
                slot.engine.outbox_running.store(false, Ordering::SeqCst);
                break;
            }
            let text = record.message.as_deref().unwrap_or_default();
            if !running {
                crate::ws::before_prompt(&slot, text, None).await;
                crate::ws::track_agent_turn(&slot);
            }
            // Dispatching is deliberately durable. After a crash or timeout
            // acceptance is unknown, so automatic replay could post twice.
            let result = if running {
                match &agent {
                    mira_acp::native::AgentHandle::AppServer(a) => {
                        a.steer(text).await.map_err(|e| e.to_string())
                    }
                    _ => Err("active steering unavailable".into()),
                }
            } else {
                agent.prompt_text(text).await.map(|_| ())
            };
            match result {
                Ok(()) => {
                    if let Some(path) = state
                        .store
                        .as_ref()
                        .and_then(|s| s.agent_log_path(&slot.id))
                    {
                        let line = serde_json::json!({ "t": mira_harness::persist::now_ms(), "driver": record.driver, "request_id": record.id, "user": {"text":text,"images":0} });
                        let _ = tokio::task::spawn_blocking(move || {
                            mira_acp::agent_sessions::append_line_to(&path, &line)
                        })
                        .await;
                    }
                    if let Err(e) = slot
                        .runtime_requests
                        .transition(&record.id, Delivery::Dispatching, Delivery::Delivered)
                        .await
                    {
                        let _ = slot.events_tx.send(crate::protocol::ServerMsg::Error {
                            text: format!(
                                "answers were sent but the receipt could not be saved: {e}"
                            ),
                        });
                    }
                }
                Err(e) => {
                    if !running {
                        crate::acp_host::AcpEventPort::for_slot(&slot)
                            .turn_ended("answer_delivery_failed");
                    }
                    let _ = slot.events_tx.send(crate::protocol::ServerMsg::Error { text: format!("answer delivery is uncertain; saved answers will not be sent again automatically: {e}") });
                }
            }
            if let Some(request) = slot
                .runtime_requests
                .snapshot()
                .await
                .into_iter()
                .find(|r| r.id == record.id)
            {
                let _ = slot
                    .events_tx
                    .send(crate::protocol::ServerMsg::RuntimeRequestUpdated { request });
            }
        }
    });
}

/// A bounded index of parked outboxes. One shared dispatcher wakes them on
/// capacity release; parked answers do not each consume a polling task.
#[derive(Default)]
pub struct RetryQueue {
    pending: Mutex<BTreeMap<String, std::sync::Weak<crate::slot::SessionSlot>>>,
    changed: tokio::sync::Notify,
}
impl RetryQueue {
    async fn park(&self, slot: &std::sync::Arc<crate::slot::SessionSlot>) {
        let mut pending = self.pending.lock().await;
        let id = slot.id.to_string();
        if pending.contains_key(&id) {
            return;
        }
        pending.retain(|_, s| s.strong_count() > 0);
        if pending.len() >= 4096 {
            let _ = slot.events_tx.send(crate::protocol::ServerMsg::Warning { text: "your answers are saved; the local retry queue is full, so reopen this chat to retry delivery".into() });
            return;
        }
        pending.insert(id, std::sync::Arc::downgrade(slot));
        self.changed.notify_one();
    }
    async fn remove(&self, id: &str) {
        self.pending.lock().await.remove(id);
    }
}

pub fn spawn_dispatcher(state: crate::state::AppState) {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = state.runtime_retries.changed.notified() => {},
                _ = crate::runtime_admission::AGENT_ADMISSION.released.notified() => {},
            }
            let slots: Vec<_> = {
                let mut pending = state.runtime_retries.pending.lock().await;
                pending.retain(|_, s| s.strong_count() > 0);
                pending
                    .values()
                    .filter_map(std::sync::Weak::upgrade)
                    .collect()
            };
            for slot in slots {
                wake_outbox(&state, &slot);
            }
        }
    });
}

#[cfg(test)]
mod delivery_recovery_tests {
    use super::*;
    use mira_acp::runtime::RuntimeQuestion;
    use mira_tools::prompt::AskUserAnswer;
    #[tokio::test]
    async fn uncertain_acceptance_survives_restart_without_automatic_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("requests.json");
        let store = RequestStore::open(Some(path.clone())).await;
        let request = RuntimeRequest {
            native_id: "item".into(),
            native_thread_id: "thread".into(),
            native_turn_id: None,
            questions: vec![RuntimeQuestion {
                id: "q".into(),
                question: "Question?".into(),
                header: None,
                options: vec![],
                required: true,
            }],
            response_capability: ResponseCapability::Message,
        };
        let record = store.insert("instance", "codex", request).await.unwrap();
        let answer = AskUserResponse {
            cancelled: false,
            answers: vec![AskUserAnswer {
                picked: vec![],
                custom: Some("answer".into()),
            }],
        };
        store.resolve(&record.id, answer.clone()).await.unwrap();
        assert!(store
            .transition(&record.id, Delivery::Queued, Delivery::Dispatching)
            .await
            .unwrap());
        let reopened = RequestStore::open(Some(path)).await;
        assert_eq!(
            reopened
                .resolve(&record.id, answer)
                .await
                .unwrap()
                .unwrap()
                .delivery,
            Delivery::Dispatching
        );
        assert!(!reopened
            .transition(&record.id, Delivery::Queued, Delivery::Dispatching)
            .await
            .unwrap());
        assert_eq!(reopened.snapshot().await.len(), 1);
    }
}
