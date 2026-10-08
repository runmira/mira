use std::sync::{Arc, Mutex as StdMutex};
use async_trait::async_trait;
use futures::{stream, stream::BoxStream, StreamExt};
use mira_ai::{ChatEvent, ChatProvider, ChatRequest, FinishReason, ProviderError};
use mira_harness::{AutoApprover, Session, SessionConfig};
use mira_policy::{Mode, Policy, PolicyConfig};
use mira_tools::{Registry, ToolContext};
use tokio::sync::{Mutex, Notify, Semaphore};

struct PausedProvider { seen: StdMutex<Vec<ChatRequest>>, started: Notify, release: Semaphore }
impl Default for PausedProvider {
    fn default() -> Self { Self { seen: StdMutex::new(Vec::new()), started: Notify::new(), release: Semaphore::new(0) } }
}
#[async_trait]
impl ChatProvider for PausedProvider {
    async fn stream(&self, request: ChatRequest) -> Result<BoxStream<'static, Result<ChatEvent, ProviderError>>, ProviderError> {
        let first = { let mut seen = self.seen.lock().unwrap(); seen.push(request); seen.len() == 1 };
        if first { self.started.notify_one(); self.release.acquire().await.unwrap().forget(); }
        Ok(stream::iter([Ok(ChatEvent::TextDelta("Reply".into())), Ok(ChatEvent::Done(FinishReason::Stop))]).boxed())
    }
}
fn session(provider: Arc<PausedProvider>, cwd: &std::path::Path) -> Session {
    Session::new(SessionConfig::new("test-model"), "sys", provider, Arc::new(Registry::new()),
        Arc::new(Mutex::new(Policy::from_config(&PolicyConfig { mode: Mode::Yolo, ..Default::default() }).unwrap())),
        Arc::new(AutoApprover { approve_asks: true }), ToolContext::new(cwd, Arc::new(mira_sandbox::Sandbox::default_scrubbed())))
}
#[tokio::test]
async fn steering_continues_the_owned_turn_and_preserves_images() {
    let tmp = tempfile::tempdir().unwrap(); let provider = Arc::new(PausedProvider::default()); let session = session(provider.clone(), tmp.path());
    let stream = session.send("Original prompt").await;
    provider.started.notified().await;
    let image = mira_core::ImageData::png("aGk=");
    let mut steer = Box::pin(session.steer("Use the other layout".into(), vec![image.clone()]));
    assert!(futures::poll!(&mut steer).is_pending());
    provider.release.add_permits(1);
    let (accepted, events) = tokio::join!(steer, stream.collect::<Vec<_>>());
    assert!(accepted); assert!(!events.is_empty());
    let seen = provider.seen.lock().unwrap(); assert_eq!(seen.len(), 2);
    let input = seen[1].messages.iter().find(|message| message.input_intent.as_deref() == Some("steer")).unwrap();
    assert_eq!(input.content.as_deref(), Some("Use the other layout")); assert_eq!(input.images, vec![image]);
    drop(seen);
    assert_eq!(session.turns().await.len(), 1);
    assert!(!session.steer("Too late".into(), vec![]).await);
}
#[tokio::test]
async fn cancellation_rejects_undelivered_steers_instead_of_losing_the_queue() {
    let tmp = tempfile::tempdir().unwrap(); let provider = Arc::new(PausedProvider::default()); let session = session(provider.clone(), tmp.path());
    let stream = session.send("Original prompt").await; provider.started.notified().await;
    let mut steer = Box::pin(session.steer("Still queued".into(), vec![]));
    assert!(futures::poll!(&mut steer).is_pending()); assert!(session.cancel().await);
    assert!(!tokio::time::timeout(std::time::Duration::from_secs(2), steer).await.unwrap());
    let _ = stream.collect::<Vec<_>>().await;
    assert!(!session.history().await.iter().any(|message| message.input_intent.is_some()));
}
