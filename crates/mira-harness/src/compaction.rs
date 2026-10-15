//! Speculative compaction. A job owns only a snapshot, never a history lock.
//! Adoption allows appended messages but rejects edits, rewinds and model changes.
use crate::{
    history::{compact_with_fallback, CompactShape},
    session::SessionConfig,
};
use mira_ai::{ChatProvider, ChatRequest, ToolSpec};
use mira_core::Message;
use std::{path::PathBuf, sync::Arc};
use tokio::task::JoinHandle;

#[derive(Default)]
pub(crate) struct BackgroundCompaction {
    pending: Option<JoinHandle<anyhow::Result<Candidate>>>,
    last_attempt: Option<(Vec<u8>, Vec<u8>)>,
}

pub(crate) struct Candidate {
    prefix: Vec<u8>,
    prefix_len: usize,
    config: Vec<u8>,
    compacted: Vec<Message>,
    removed: Vec<Message>,
}

pub(crate) fn native_request(cfg: &SessionConfig, tools: Vec<ToolSpec>) -> ChatRequest {
    ChatRequest {
        model: cfg.model.clone(),
        messages: vec![],
        tools,
        temperature: cfg.temperature,
        max_tokens: Some(cfg.max_tokens.unwrap_or(16_000).max(8_000)),
        reasoning_effort: cfg.reasoning_effort.clone(),
        service_tier: cfg.service_tier.clone(),
        response_format: None,
    }
}

fn config_key(cfg: &SessionConfig, tools: &[ToolSpec]) -> Vec<u8> {
    serde_json::to_vec(&(cfg, tools)).expect("session config is serializable")
}

impl Drop for BackgroundCompaction {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl BackgroundCompaction {
    pub fn cancel(&mut self) {
        if let Some(job) = self.pending.take() {
            job.abort();
        }
    }

    pub fn start(
        &mut self,
        history: Vec<Message>,
        cfg: &SessionConfig,
        provider: Arc<dyn ChatProvider>,
        cwd: PathBuf,
        tools: Vec<ToolSpec>,
    ) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let prefix = serde_json::to_vec(&history).expect("history is serializable");
        let config = config_key(cfg, &tools);
        if self.last_attempt.as_ref() == Some(&(prefix.clone(), config.clone())) {
            return false;
        }
        self.last_attempt = Some((prefix.clone(), config.clone()));
        let cfg = cfg.clone();
        self.pending = Some(tokio::spawn(async move {
            let prefix_len = history.len();
            let mut compacted = history;
            let native = native_request(&cfg, tools);
            let removed = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                compact_with_fallback(
                    &mut compacted,
                    provider.as_ref(),
                    &cfg.model,
                    &cfg.background_model(cfg.compactor_model.as_deref()),
                    &cwd,
                    CompactShape {
                        native_request: Some(&native),
                        speculative: true,
                        ..Default::default()
                    },
                ),
            )
            .await??;
            Ok(Candidate {
                prefix,
                prefix_len,
                config,
                compacted,
                removed,
            })
        }));
        true
    }

    pub async fn take_ready(&mut self) -> Option<Candidate> {
        if !self.pending.as_ref()?.is_finished() {
            return None;
        }
        match self.pending.take()?.await {
            Ok(Ok(candidate)) => Some(candidate),
            Ok(Err(error)) => {
                tracing::debug!(%error,"background compaction failed; keeping history");
                None
            }
            Err(_) => None,
        }
    }
}

impl Candidate {
    pub fn adopt(
        mut self,
        history: &mut Vec<Message>,
        cfg: &SessionConfig,
        tools: &[ToolSpec],
    ) -> Option<Vec<Message>> {
        if self.config != config_key(cfg, tools)
            || history.len() < self.prefix_len
            || serde_json::to_vec(&history[..self.prefix_len]).ok()? != self.prefix
        {
            return None;
        }
        self.compacted.extend(history.drain(self.prefix_len..));
        *history = self.compacted;
        Some(self.removed)
    }
}

/// Starting early overlaps work with the foreground; post-turn preparation
/// starts later to avoid summarizing ordinary short conversations.
pub(crate) fn due(tokens: u64, window: u64, post_turn: bool) -> bool {
    let percent = if post_turn { 80 } else { 70 };
    tokens >= window.saturating_mul(percent) / 100
        && (post_turn || tokens < crate::history::auto_compact_at(window))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(history: &[Message], cfg: &SessionConfig) -> Candidate {
        Candidate {
            prefix: serde_json::to_vec(history).unwrap(),
            prefix_len: history.len(),
            config: config_key(cfg, &[]),
            compacted: vec![Message::user("summary")],
            removed: history.to_vec(),
        }
    }
    #[test]
    fn candidate_preserves_appends_and_rejects_rewinds_edits_and_model_changes() {
        let cfg = SessionConfig::new("model");
        let original = vec![Message::user("old"), Message::assistant("answer")];
        let mut appended = original.clone();
        appended.push(Message::user("new"));
        assert!(candidate(&original, &cfg)
            .adopt(&mut appended, &cfg, &[])
            .is_some());
        assert_eq!(appended[1].content.as_deref(), Some("new"));
        let mut rewound = original[..1].to_vec();
        assert!(candidate(&original, &cfg)
            .adopt(&mut rewound, &cfg, &[])
            .is_none());
        let mut edited = original.clone();
        edited[0].content = Some("edited".into());
        assert!(candidate(&original, &cfg)
            .adopt(&mut edited, &cfg, &[])
            .is_none());
        let mut changed = cfg.clone();
        changed.model = "different".into();
        assert!(candidate(&original, &cfg)
            .adopt(&mut original.clone(), &changed, &[])
            .is_none());
    }
    struct WaitingProvider {
        started: Arc<tokio::sync::Notify>,
        dropped: Arc<tokio::sync::Notify>,
    }
    #[async_trait::async_trait]
    impl ChatProvider for WaitingProvider {
        async fn stream(
            &self,
            _: mira_ai::ChatRequest,
        ) -> Result<
            futures::stream::BoxStream<'static, Result<mira_ai::ChatEvent, mira_ai::ProviderError>>,
            mira_ai::ProviderError,
        > {
            struct Guard(Arc<tokio::sync::Notify>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.notify_one();
                }
            }
            let _guard = Guard(self.dropped.clone());
            self.started.notify_one();
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn cancellation_aborts_the_owned_request_and_deduplicates_attempts() {
        let started = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(tokio::sync::Notify::new());
        let provider: Arc<dyn ChatProvider> = Arc::new(WaitingProvider {
            started: started.clone(),
            dropped: dropped.clone(),
        });
        let cfg = SessionConfig::new("m");
        let h = vec![Message::user("request"), Message::assistant("answer")];
        let mut state = BackgroundCompaction::default();
        assert!(state.start(
            h.clone(),
            &cfg,
            provider.clone(),
            PathBuf::from("."),
            vec![]
        ));
        tokio::time::timeout(std::time::Duration::from_secs(2), started.notified())
            .await
            .unwrap();
        assert!(!state.start(
            h.clone(),
            &cfg,
            provider.clone(),
            PathBuf::from("."),
            vec![]
        ));
        state.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(2), dropped.notified())
            .await
            .unwrap();
        assert!(!state.start(h, &cfg, provider, PathBuf::from("."), vec![]));
        assert!(state.take_ready().await.is_none());
    }

    #[test]
    fn thresholds_leave_foreground_overflow_recovery_in_charge() {
        assert!(!due(139_999, 200_000, false));
        assert!(due(140_000, 200_000, false));
        assert!(!due(159_999, 200_000, true));
        assert!(due(160_000, 200_000, true));
        assert!(!due(180_000, 200_000, false));
        assert!(due(180_000, 200_000, true));
    }
}
