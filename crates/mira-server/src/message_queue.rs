//! Persistent per-chat input outbox. Ambiguous delivery is held for review,
//! never silently replayed. Workers exist only for chats with queued input.
use crate::{protocol::ServerMsg, slot::SessionSlot, AppState};
use mira_core::ImageData;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<LimitRecovery>,
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub images: Vec<ImageData>,
    pub engine: Option<String>,
    #[serde(default)]
    pub dispatching: bool,
    #[serde(default)]
    pub delivered: bool,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub error: Option<String>,
}
/// A recovery is inert until explicitly scheduled. Persisted with the outbox;
/// claiming it uses the same durable, fail-closed delivery receipt as any input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LimitRecovery {
    #[serde(default)]
    pub model: Option<String>,
    pub created_at: u64,
    pub reset_at: Option<u64>,
    pub scheduled_at: Option<u64>,
    pub snoozed: bool,
}
fn now_ms() -> u64 { mira_harness::persist::now_ms() }
fn eligible(item: &QueuedInput, now: u64) -> bool {
    !item.delivered && item.recovery.as_ref().is_none_or(|r| r.scheduled_at.is_some_and(|at| at <= now))
}
pub struct MessageQueue {
    pub limit_reset: std::sync::Mutex<Option<u64>>,
    path: Option<PathBuf>,
    items: Mutex<Vec<QueuedInput>>,
    load_error: Option<String>,
    pub worker: AtomicBool,
    closed: AtomicBool,
    changed: tokio::sync::Notify,
}
impl MessageQueue {
    pub async fn open(path: Option<PathBuf>) -> Self {
        let loaded = match &path {
            Some(path) => match tokio::fs::read(path).await {
                Ok(bytes) => {
                    serde_json::from_slice::<Vec<QueuedInput>>(&bytes).map_err(|e| e.to_string())
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
                Err(e) => Err(e.to_string()),
            },
            None => Ok(vec![]),
        };
        let (mut items, load_error) = match loaded {
            Ok(items) => (items, None),
            Err(e) => (vec![], Some(e)),
        };
        for item in &mut items {
            if item.dispatching {
                item.error =
                    Some("Delivery was interrupted. Check the transcript before resending.".into());
            }
        }
        Self {
            limit_reset: std::sync::Mutex::new(None),
            path,
            items: Mutex::new(items),
            load_error,
            worker: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        }
    }
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.changed.notify_one();
        let _guard = self.items.lock().await;
    }
    pub fn reopen(&self) {
        self.closed.store(false, Ordering::SeqCst);
    }
    async fn commit(&self, items: &[QueuedInput]) -> Result<(), String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("The chat queue is closed".into());
        }
        if let Some(error) = &self.load_error {
            return Err(format!("Could not read queued messages: {error}"));
        }
        let Some(path) = self.path.clone() else {
            return Err("This chat does not have persistent queue storage".into());
        };
        let bytes = serde_json::to_vec(items).map_err(|e| e.to_string())?;
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            use std::io::Write;
            let parent = path
                .parent()
                .ok_or_else(|| std::io::Error::other("queue path has no parent"))?;
            std::fs::create_dir_all(parent)?;
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                let mut file = std::fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temp)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
                std::fs::rename(&temp, &path)?;
                std::fs::File::open(parent)?.sync_all()
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
    pub async fn snapshot(&self) -> Vec<QueuedInput> {
        self.items
            .lock()
            .await
            .iter()
            .filter(|i| !i.delivered)
            .cloned()
            .collect()
    }
    pub async fn enqueue(&self, mut item: QueuedInput) -> Result<(), String> {
        use sha2::{Digest, Sha256};
        item.fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&item.text, &item.images, &item.engine))
                    .map_err(|e| e.to_string())?
            )
        );
        let mut guard = self.items.lock().await;
        if let Some(existing) = guard.iter().find(|i| i.id == item.id) {
            if existing.fingerprint == item.fingerprint {
                return Ok(());
            }
            return Err("Queue message ID already exists with different content".into());
        }
        if item.id.is_empty() || item.id.len() > 128 || item.text.len() > 1024 * 1024 {
            return Err("Invalid queued input".into());
        }
        let total: usize = guard
            .iter()
            .flat_map(|i| &i.images)
            .map(|i| i.data.len())
            .sum();
        if guard.iter().filter(|i| !i.delivered).count() >= 100
            || total + item.images.iter().map(|i| i.data.len()).sum::<usize>() > 64 * 1024 * 1024
        {
            return Err("The chat queue is full".into());
        }
        let mut next = guard.clone();
        next.push(item);
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    pub async fn remove(&self, id: &str) -> Result<(), String> {
        let mut guard = self.items.lock().await;
        if guard
            .iter()
            .any(|i| i.id == id && i.dispatching && i.error.is_none())
        {
            return Err("This message is already being delivered".into());
        }
        let mut next = guard.clone();
        if let Some(item) = next.iter_mut().find(|i| i.id == id) {
            item.delivered = true;
            item.dispatching = false;
            item.error = None;
            item.text.clear();
            item.images.clear();
        }
        while next.iter().filter(|i| i.delivered).count() > 200 {
            if let Some(index) = next.iter().position(|i| i.delivered) {
                next.remove(index);
            }
        }
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    pub async fn edit(&self, id: &str, expected: &str, text: String, images: Vec<ImageData>) -> Result<(), String> {
        if text.len() > 1024 * 1024 || (text.trim().is_empty() && images.is_empty()) {
            return Err("Queued message must contain text or an attachment".into());
        }
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        let index = next.iter().position(|i| i.id == id && !i.delivered)
            .ok_or("This queued message has already started or was removed")?;
        let item = &next[index];
        if item.recovery.is_some() { return Err("Use the recovery controls for this continuation".into()); }
        if item.dispatching || item.error.is_some() {
            return Err("This message is being delivered or needs delivery review".into());
        }
        if expected.is_empty() || item.fingerprint != expected {
            return Err("This queued message changed elsewhere. Reopen the editor to get the latest version.".into());
        }
        let total: usize = next.iter().enumerate().filter(|(i, _)| *i != index)
            .flat_map(|(_, i)| &i.images).chain(images.iter()).map(|i| i.data.len()).sum();
        if total > 64 * 1024 * 1024 { return Err("The chat queue is full".into()); }
        let item = &mut next[index];
        item.text = text;
        item.images = images;
        use sha2::{Digest, Sha256};
        item.fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&(&item.text, &item.images, &item.engine)).map_err(|e| e.to_string())?));
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    /// Move one pending item without replacing the queue from a stale client snapshot.
    pub async fn reorder(&self, id: &str, before_id: Option<&str>) -> Result<(), String> {
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        let index = next.iter().position(|i| i.id == id && !i.delivered)
            .ok_or("This queued message has already started or was removed")?;
        if next[index].recovery.is_some() { return Err("Use the recovery controls for this continuation".into()); }
        if next[index].dispatching || next[index].error.is_some() {
            return Err("This message is being delivered or needs delivery review".into());
        }
        if before_id == Some(id) { return Ok(()); }
        if let Some(target) = before_id {
            if !next.iter().any(|i| i.id == target && !i.delivered && !i.dispatching && i.error.is_none()) {
                return Err("The target queued message is no longer available".into());
            }
        }
        let item = next.remove(index);
        let target = before_id.and_then(|id| next.iter().position(|i| i.id == id)).unwrap_or(next.len());
        next.insert(target, item);
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    pub async fn claim(&self, id: &str, engine: Option<&str>) -> Result<QueuedInput, String> {
        self.claim_inner(id, engine, false).await?.ok_or_else(|| "Queued message no longer exists".into())
    }
    async fn claim_first(&self, id: &str, engine: Option<&str>) -> Result<Option<QueuedInput>, String> {
        self.claim_inner(id, engine, true).await
    }
    async fn claim_inner(&self, id: &str, engine: Option<&str>, first_only: bool) -> Result<Option<QueuedInput>, String> {
        let mut guard = self.items.lock().await;
        // A reorder/removal can happen between the worker snapshot and this lock.
        if first_only {
            let first = guard.iter().find(|i| eligible(i, now_ms()));
            if !first.is_some_and(|i| i.id == id && !i.dispatching && i.error.is_none()) {
                return Ok(None);
            }
        }
        let mut next = guard.clone();
        let item = next
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or("Queued message no longer exists")?;
        if item.delivered || item.dispatching || item.error.is_some() {
            return Err("Message delivery is already pending or needs review".into());
        }
        if item.engine.as_deref() != engine {
            return Err("The chat engine changed. Remove this queued message and send it to the new engine.".into());
        }
        if !eligible(item, now_ms()) { return Err("Recovery is not scheduled yet".into()); }
        item.dispatching = true;
        let item = item.clone();
        self.commit(&next).await?;
        *guard = next;
        Ok(Some(item))
    }
    pub async fn validate_recovery(&self, id: &str, engine: Option<&str>, model: Option<&str>) -> Result<(), String> {
        let guard = self.items.lock().await;
        let Some(item) = guard.iter().find(|i| i.id == id && i.recovery.is_some()) else { return Ok(()); };
        if item.delivered || item.error.is_some() || item.engine.as_deref() != engine || item.recovery.as_ref().and_then(|r| r.model.as_deref()) != model { return Err("Recovery was superseded or the engine/model changed. Check the transcript before retrying.".into()); }
        Ok(())
    }
    pub async fn offer_recovery(&self, engine: Option<String>, model: Option<String>, reset_at: Option<u64>) -> Result<(), String> {
        // Do not replace an already-claimed recovery (including an uncertain receipt).
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        next.retain(|i| i.recovery.is_none() || i.dispatching);
        next.push(QueuedInput {
            id: format!("recovery-{}", uuid::Uuid::new_v4()),
            text: "Continue the interrupted task from where you stopped. Check existing results before repeating any actions.".into(),
            images: vec![], engine, dispatching: false, delivered: false,
            fingerprint: String::new(), error: None,
            recovery: Some(LimitRecovery { model, created_at: now_ms(), reset_at, scheduled_at: None, snoozed: false }),
        });
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    pub async fn update_recovery(&self, id: &str, action: &str) -> Result<(), String> {
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        let item = next.iter_mut().find(|i| i.id == id && !i.delivered).ok_or("Recovery no longer exists")?;
        if item.recovery.is_none() { return Err("This is not a recovery".into()); }
        if action == "dismiss" && (item.error.is_some() || !item.dispatching) {
            item.delivered = true; item.text.clear(); item.images.clear();
            self.commit(&next).await?; *guard = next; self.changed.notify_one(); return Ok(());
        }
        if item.dispatching || item.error.is_some() { return Err("Delivery already started. Check the transcript before retrying.".into()); }
        let recovery = item.recovery.as_mut().ok_or("This is not a recovery")?;
        match action {
            "schedule" => { recovery.scheduled_at = Some(recovery.reset_at.filter(|at| *at > now_ms()).ok_or("No future reset was reported; retry manually")?.saturating_add(1000)); recovery.snoozed = false; }
            "retry" => { recovery.scheduled_at = Some(now_ms()); recovery.snoozed = false; }
            "cancel" => { recovery.scheduled_at = None; }
            "snooze" => { if recovery.scheduled_at.is_some() { return Err("Cancel auto-resume before snoozing".into()); } recovery.snoozed = true; }
            "show" => { recovery.snoozed = false; }
            _ => return Err("Unknown recovery action".into()),
        }
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    /// A manual prompt supersedes an unclaimed continuation. Never erase an
    /// uncertain/claimed receipt, which must remain inspectable after a crash.
    pub async fn supersede_recovery(&self, except: Option<&str>) -> Result<(), String> {
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        let mut superseded = false;
        for item in &mut next {
            if item.recovery.is_some() && item.dispatching && !item.delivered && except != Some(item.id.as_str()) && item.error.is_none() { superseded = true; item.error = Some("A newer message superseded this continuation. Check the transcript.".into()); }
        }
        next.retain(|i| i.recovery.is_none() || i.dispatching || except == Some(i.id.as_str()));
        if !superseded && next.len() == guard.len() { return Ok(()); }
        self.commit(&next).await?;
        *guard = next;
        self.changed.notify_one();
        Ok(())
    }
    pub async fn settle(&self, id: &str, error: Option<String>) -> Result<(), String> {
        let mut guard = self.items.lock().await;
        let mut next = guard.clone();
        if let Some(error) = error {
            if let Some(item) = next.iter_mut().find(|i| i.id == id) {
                item.error = Some(error);
            }
        } else {
            if let Some(item) = next.iter_mut().find(|i| i.id == id) {
                item.delivered = true;
                item.dispatching = false;
                item.text.clear();
                item.images.clear();
            }
        }
        // Retain bounded receipts to make retries after a lost acknowledgement idempotent.
        while next.iter().filter(|i| i.delivered).count() > 200 {
            if let Some(index) = next.iter().position(|i| i.delivered) {
                next.remove(index);
            }
        }
        self.commit(&next).await?;
        *guard = next;
        Ok(())
    }
}
pub fn observe_limits(slot: &SessionSlot, frame: &ServerMsg) {
    let reset = match frame {
        ServerMsg::RateLimit { rate_limit, .. } => [rate_limit.requests, rate_limit.tokens, rate_limit.input_tokens, rate_limit.output_tokens].into_iter().flatten().filter(|b| b.remaining == Some(0)).filter_map(|b| b.reset_secs).max().map(|secs| now_ms().saturating_add(secs.saturating_mul(1000))),
        ServerMsg::AcpLimits { windows, .. } => windows.iter().filter(|w| w.utilization >= 1.0).filter_map(|w| w.resets_at).filter(|at| *at > 0).max().map(|at| (at as u64).saturating_mul(1000)),
        _ => return,
    };
    *slot.message_queue.limit_reset.lock().unwrap_or_else(|e| e.into_inner()) = reset;
}
pub async fn offer_recovery(slot: &Arc<SessionSlot>, explicit_reset: Option<&str>) {
    let reported = explicit_reset.and_then(|v| v.parse::<u64>().ok().map(|at| if at < 100_000_000_000 {at.saturating_mul(1000)} else {at})
        .or_else(|| chrono::DateTime::parse_from_rfc3339(v).ok().and_then(|at| u64::try_from(at.timestamp_millis()).ok())));
    let reset = reported.or(*slot.message_queue.limit_reset.lock().unwrap_or_else(|e|e.into_inner()));
    let (engine, model, _) = slot.selection.snapshot();
    if let Err(text) = slot.message_queue.offer_recovery(engine, model, reset).await { let _ = slot.events_tx.send(ServerMsg::Warning { text: format!("Could not save usage recovery: {text}") }); }
    // Settings → "Auto resume after limit reset": schedule it as if the user
    // had pressed the button. Without a known reset time there is nothing to
    // schedule, so it stays a manual choice.
    let auto = mira_config::MiraConfig::load_global().map(|c| c.sessions.auto_resume()).unwrap_or(false);
    if auto && reset.is_some_and(|at| at > now_ms()) {
        let id = slot.message_queue.snapshot().await.into_iter()
            .find(|i| i.recovery.as_ref().is_some_and(|r| r.scheduled_at.is_none()) && !i.delivered && !i.dispatching)
            .map(|i| i.id);
        if let Some(id) = id {
            match slot.message_queue.update_recovery(&id, "schedule").await {
                Ok(()) => {
                    if let Some(state) = STATE.get() { wake(state, slot); }
                }
                Err(text) => { let _ = slot.events_tx.send(ServerMsg::Warning { text: format!("Could not schedule auto-resume: {text}") }); }
            }
        }
    }
    publish(slot).await;
}
/// The server's state, for paths that schedule queue work without a handle
/// to it (a usage-limit turn end). Set once at startup by [`recover`].
static STATE: std::sync::OnceLock<AppState> = std::sync::OnceLock::new();
pub fn recover(state: AppState) {
    let _ = STATE.set(state.clone());
    tokio::spawn(async move {
        let Some(store) = state.store.as_ref() else {
            return;
        };
        match store.queued_sessions().await {
            Ok(ids) => {
                for id in ids {
                    if let Ok(slot) = state.ensure_slot(&id).await {
                        wake(&state, &slot);
                    }
                }
            }
            Err(error) => tracing::warn!(%error,"could not recover queued input"),
        }
    });
}
pub async fn publish(slot: &SessionSlot) {
    let _ = slot.events_tx.send(ServerMsg::QueueUpdated {
        session_id: slot.id.to_string(),
        items: slot.message_queue.snapshot().await,
    });
}
/// One event-driven worker per nonempty outbox; no timers per idle session.
pub fn wake(state: &AppState, slot: &Arc<SessionSlot>) {
    if slot.message_queue.worker.swap(true, Ordering::SeqCst) {
        return;
    }
    let state = state.clone();
    let slot = slot.clone();
    tokio::spawn(async move {
        let mut events = slot.events_tx.subscribe();
        loop {
            let activity_wake = slot.engine.activity_changed.notified();
            tokio::pin!(activity_wake);
            activity_wake.as_mut().enable();
            let items = slot.message_queue.snapshot().await;
            let Some(item) = items.iter().find(|i| eligible(i, now_ms())).cloned() else {
                let next_at = items.iter().filter(|i| !i.dispatching && i.error.is_none()).filter_map(|i| i.recovery.as_ref()?.scheduled_at).min();
                let Some(next_at) = next_at else { break; };
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(next_at.saturating_sub(now_ms()))) => {},
                    _ = slot.message_queue.changed.notified() => {},
                    _ = &mut activity_wake => {},
                }
                continue;
            };
            if slot.engine.retired.load(Ordering::SeqCst)
                || slot.message_queue.closed.load(Ordering::SeqCst)
                || item.error.is_some()
                || item.dispatching
            {
                break;
            }
            if slot.is_foreground_running().await {
                let frame = tokio::select! {
                    frame = events.recv() => frame,
                    _ = slot.message_queue.changed.notified() => continue,
                    _ = &mut activity_wake => continue,
                };
                match frame {
                    Ok(ServerMsg::Done | ServerMsg::AcpTurnEnd { .. }) => {
                        // Done is emitted just before the harness task drops its handle.
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => {}
                }
                continue;
            }
            let (engine, _, _) = slot.selection.snapshot();
            let claimed = match slot.message_queue.claim_first(&item.id, engine.as_deref()).await {
                Ok(Some(item)) => item,
                Ok(None) => continue,
                Err(error) => {
                    let _ = slot.message_queue.settle(&item.id, Some(error)).await;
                    publish(&slot).await;
                    break;
                }
            };
            publish(&slot).await;
            let _ = slot.events_tx.send(ServerMsg::QueueDelivery {
                session_id: slot.id.to_string(),
                item: claimed.clone(),
            });
            if let Err(error) = crate::ws::send_input(
                &state,
                &slot,
                claimed.text,
                claimed.images,
                Some(item.id.clone()),
            )
            .await
            {
                let _ = slot.message_queue.settle(&item.id, Some(error)).await;
                publish(&slot).await;
                break;
            }
            // Keep the durable dispatch marker until a terminal frame is observed.
            let error = loop {
                let frame = tokio::select! {
                    frame = events.recv() => frame,
                    _ = slot.message_queue.changed.notified() => {
                        if slot.message_queue.closed.load(Ordering::SeqCst) { break Some("Chat was deleted during delivery".into()); }
                        continue;
                    },
                };
                match frame {
                    Ok(ServerMsg::Error { text }) => break Some(text),
                    Ok(ServerMsg::Done | ServerMsg::AcpTurnEnd { .. }) => break None,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        break Some(
                            "Delivery outcome could not be confirmed. Check the transcript.".into(),
                        )
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        break Some("Runtime disconnected during delivery".into())
                    }
                    _ => {}
                }
            };
            if let Err(error) = slot.message_queue.settle(&item.id, error).await {
                let _ = slot.events_tx.send(ServerMsg::Error { text: error });
                break;
            }
            publish(&slot).await;
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        slot.message_queue.worker.store(false, Ordering::SeqCst);
        if slot
            .message_queue
            .snapshot()
            .await
            .iter().any(|i| !i.dispatching && i.error.is_none() && (eligible(i, now_ms()) || i.recovery.as_ref().is_some_and(|r| r.scheduled_at.is_some())))
            && !slot.message_queue.closed.load(Ordering::SeqCst)
            && !slot.engine.retired.load(Ordering::SeqCst)
        {
            wake(&state, &slot);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str) -> QueuedInput {
        QueuedInput {
            id: id.into(),
            text: "continue".into(),
            images: vec![],
            engine: Some("codex".into()),
            dispatching: false,
            delivered: false,
            fingerprint: String::new(),
            error: None,
            recovery: None,
        }
    }
    #[tokio::test]
    async fn recovery_controls_cannot_dismiss_an_ordinary_message() {
        let dir = tempfile::tempdir().unwrap();
        let queue = MessageQueue::open(Some(dir.path().join("r.queue"))).await;
        queue.enqueue(item("ordinary")).await.unwrap();
        assert!(queue.update_recovery("ordinary", "dismiss").await.is_err());
        assert_eq!(queue.snapshot().await.len(), 1);
    }
    #[tokio::test]
    async fn continuation_is_bound_to_engine_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let queue = MessageQueue::open(Some(dir.path().join("r.queue"))).await;
        queue.offer_recovery(Some("codex".into()), Some("original-model".into()), None).await.unwrap();
        let id = queue.snapshot().await[0].id.clone();
        queue.update_recovery(&id, "retry").await.unwrap();
        queue.claim(&id, Some("codex")).await.unwrap();
        assert!(queue.validate_recovery(&id, Some("codex"), Some("original-model")).await.is_ok());
        assert!(queue.validate_recovery(&id, Some("opencode"), Some("original-model")).await.is_err());
        assert!(queue.validate_recovery(&id, Some("codex"), Some("new-model")).await.is_err());
    }

    #[tokio::test]
    async fn limit_recovery_is_inert_durable_and_cancellable() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("recovery.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        queue.offer_recovery(Some("codex".into()), Some("model-a".into()), Some(now_ms()+60_000)).await.unwrap();
        let id = queue.snapshot().await[0].id.clone();
        assert!(queue.claim(&id, Some("codex")).await.is_err());
        queue.enqueue(item("normal")).await.unwrap();
        assert!(queue.claim_first("normal", Some("codex")).await.unwrap().is_some());
        queue.settle("normal", None).await.unwrap();
        queue.update_recovery(&id, "schedule").await.unwrap(); drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        assert!(queue.snapshot().await[0].recovery.as_ref().unwrap().scheduled_at.is_some());
        assert!(queue.claim(&id, Some("codex")).await.is_err());
        queue.update_recovery(&id, "cancel").await.unwrap();
        assert!(queue.snapshot().await[0].recovery.as_ref().unwrap().scheduled_at.is_none());
        queue.update_recovery(&id, "snooze").await.unwrap();
        assert!(queue.snapshot().await[0].recovery.as_ref().unwrap().snoozed);
    }
    #[tokio::test]
    async fn unknown_reset_is_manual_and_claimed_recovery_never_replays() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("recovery.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        queue.offer_recovery(Some("codex".into()), Some("model-a".into()), None).await.unwrap();
        let id = queue.snapshot().await[0].id.clone();
        assert!(queue.update_recovery(&id, "schedule").await.is_err());
        queue.update_recovery(&id, "retry").await.unwrap();
        queue.claim(&id, Some("codex")).await.unwrap();
        assert!(queue.validate_recovery(&id, Some("codex"), Some("model-b")).await.is_err());
        assert!(queue.update_recovery(&id, "cancel").await.is_err());
        drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        assert!(queue.snapshot().await[0].error.is_some());
        assert!(queue.update_recovery(&id, "retry").await.is_err());
        queue.update_recovery(&id, "dismiss").await.unwrap();
        assert!(queue.snapshot().await.is_empty());
    }
    #[tokio::test]
    async fn newer_input_fences_even_an_already_claimed_continuation() {
        let dir = tempfile::tempdir().unwrap(); let queue = MessageQueue::open(Some(dir.path().join("r.queue"))).await;
        queue.offer_recovery(Some("codex".into()), None, None).await.unwrap();
        let id = queue.snapshot().await[0].id.clone();
        queue.update_recovery(&id, "retry").await.unwrap(); queue.claim(&id, Some("codex")).await.unwrap();
        queue.supersede_recovery(None).await.unwrap();
        assert!(queue.validate_recovery(&id, Some("codex"), None).await.is_err());
        queue.offer_recovery(Some("codex".into()), None, None).await.unwrap();
        queue.supersede_recovery(None).await.unwrap();
        assert_eq!(queue.snapshot().await.len(), 1); // only the held receipt remains
    }
    #[tokio::test]
    async fn durable_queue_recovers_and_deduplicates_delivered_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        queue.enqueue(item("one")).await.unwrap();
        drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        assert_eq!(queue.snapshot().await.len(), 1);
        queue.claim("one", Some("codex")).await.unwrap();
        assert!(queue.claim("one", Some("codex")).await.is_err());
        queue.settle("one", None).await.unwrap();
        queue.enqueue(item("one")).await.unwrap();
        assert!(queue.snapshot().await.is_empty());
        let mut changed = item("one");
        changed.text = "different".into();
        assert!(queue.enqueue(changed).await.is_err());
    }
    #[tokio::test]
    async fn restart_holds_uncertain_delivery_and_engine_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        queue.enqueue(item("one")).await.unwrap();
        assert!(queue.claim("one", Some("claude")).await.is_err());
        queue.claim("one", Some("codex")).await.unwrap();
        drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        assert!(queue.snapshot().await[0].error.is_some());
        assert!(queue.claim("one", Some("codex")).await.is_err());
        queue.remove("one").await.unwrap();
        assert!(queue.snapshot().await.is_empty());
    }
    #[tokio::test]
    async fn corrupt_storage_and_deleted_chats_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.queue");
        tokio::fs::write(&path, b"broken").await.unwrap();
        let queue = MessageQueue::open(Some(path.clone())).await;
        assert!(queue.enqueue(item("one")).await.is_err());
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
        let queue = MessageQueue::open(Some(dir.path().join("second.queue"))).await;
        queue.close().await;
        assert!(queue.enqueue(item("one")).await.is_err());
    }
    #[tokio::test]
    async fn edits_keep_position_and_attachments_and_reject_stale_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        let mut original = item("one");
        original.images.push(ImageData { media_type: "image/png".into(), data: "image-data".into(), source: None });
        queue.enqueue(original).await.unwrap();
        queue.enqueue(item("two")).await.unwrap();
        let before = queue.snapshot().await.remove(0);
        let body = "## Attached files\n\n### file.rs\n```rust\nfn main() {}\n```\n\nChanged prompt";
        queue.edit("one", &before.fingerprint, body.into(), before.images.clone()).await.unwrap();
        assert!(queue.edit("one", &before.fingerprint, "stale".into(), vec![]).await.is_err());
        drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        let saved = queue.snapshot().await;
        assert_eq!(saved.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["one", "two"]);
        assert_eq!(saved[0].text, body);
        assert_eq!(saved[0].images[0].data, "image-data");
        queue.claim("one", Some("codex")).await.unwrap();
        assert!(queue.edit("one", &saved[0].fingerprint, "too late".into(), vec![]).await.is_err());
        assert_eq!(queue.snapshot().await[0].text, body);
    }
    #[tokio::test]
    async fn reordering_is_durable_and_fences_stale_worker_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chat.queue");
        let queue = MessageQueue::open(Some(path.clone())).await;
        for id in ["one", "two", "three"] { queue.enqueue(item(id)).await.unwrap(); }
        queue.reorder("three", Some("one")).await.unwrap();
        assert!(queue.claim_first("one", Some("codex")).await.unwrap().is_none());
        assert!(queue.reorder("two", Some("missing")).await.is_err());
        queue.reorder("one", None).await.unwrap();
        drop(queue);
        let queue = MessageQueue::open(Some(path)).await;
        assert_eq!(queue.snapshot().await.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["three", "two", "one"]);
        queue.claim_first("three", Some("codex")).await.unwrap().unwrap();
        assert!(queue.claim_first("three", Some("codex")).await.unwrap().is_none());
        assert!(queue.reorder("three", None).await.is_err());
        assert!(queue.reorder("one", Some("three")).await.is_err());
    }
}
