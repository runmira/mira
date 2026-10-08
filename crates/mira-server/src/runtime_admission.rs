//! Bounded local execution capacity. Persisted sessions are not processes:
//! each resident CLI runtime holds a lease until shutdown. Admission never
//! creates an unbounded queue of tasks blocked on semaphore acquisition.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub struct Admission {
    pub released: Arc<tokio::sync::Notify>,
    global: Arc<Semaphore>,
    drivers: Mutex<HashMap<String, Arc<Semaphore>>>,
    per_driver: usize,
}
pub struct RuntimeLease {
    _global: Option<OwnedSemaphorePermit>,
    _driver: Option<OwnedSemaphorePermit>,
    released: Arc<tokio::sync::Notify>,
}
impl Drop for RuntimeLease {
    fn drop(&mut self) {
        self._global.take();
        self._driver.take();
        self.released.notify_one();
    }
}
impl Admission {
    pub fn new(global: usize, per_driver: usize) -> Self {
        Self {
            released: Arc::new(tokio::sync::Notify::new()),
            global: Arc::new(Semaphore::new(global.max(1))),
            drivers: Mutex::new(HashMap::new()),
            per_driver: per_driver.max(1),
        }
    }
    pub fn acquire(&self, driver: &str) -> Result<RuntimeLease, String> {
        let local = self
            .drivers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(driver.to_owned())
            .or_insert_with(|| Arc::new(Semaphore::new(self.per_driver)))
            .clone();
        let driver_permit = local.try_acquire_owned().map_err(|_| format!("{driver} runtime capacity reached; retry after another session stops or releases its idle agent"))?;
        let global = self.global.clone().try_acquire_owned().map_err(|_| "local agent runtime capacity reached; retry after another session releases its agent".to_string())?;
        Ok(RuntimeLease {
            _global: Some(global),
            _driver: Some(driver_permit),
            released: self.released.clone(),
        })
    }
}
fn configured_limit(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 1000000)
        .unwrap_or(default)
}
pub static AGENT_ADMISSION: once_cell::sync::Lazy<Admission> = once_cell::sync::Lazy::new(|| {
    Admission::new(
        configured_limit("MIRA_MAX_AGENT_RUNTIMES", 64),
        configured_limit("MIRA_MAX_AGENT_RUNTIMES_PER_PROVIDER", 16),
    )
});

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn providers_are_independent_and_leases_return_on_failure_or_shutdown() {
        let a = Admission::new(2, 1);
        let codex = a.acquire("codex").unwrap();
        assert!(a.acquire("codex").is_err());
        let claude = a.acquire("claude").unwrap();
        assert!(a.acquire("grok").is_err());
        drop(codex);
        let _grok = a.acquire("grok").unwrap();
        drop(claude);
        assert!(a.acquire("claude").is_ok());
    }
    #[test]
    fn a_million_admission_attempts_do_not_create_waiters() {
        let a = Admission::new(1, 1);
        let _lease = a.acquire("codex").unwrap();
        for _ in 0..1000000 {
            assert!(a.acquire("codex").is_err());
        }
        assert_eq!(a.drivers.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    #[tokio::test]
    async fn release_notification_observes_available_capacity() {
        let admission = Admission::new(1, 1);
        let lease = admission.acquire("codex").unwrap();
        let released = admission.released.notified();
        drop(lease);
        released.await;
        assert!(admission.acquire("codex").is_ok());
    }
}
