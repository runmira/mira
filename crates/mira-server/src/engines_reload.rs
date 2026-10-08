//! Rebuild the engine registry when `~/.mira/mira.yaml` changes (#84).
//!
//! Adding an `engines:` instance, a `providers:` entry or an API key used to
//! need a server restart before pickers or a model switch saw it. Now the
//! config file is watched: on a change the registry is rebuilt, provider
//! pools (the server's and every open chat's) gain new instances, drop
//! removed ones and pick up changed keys, a selection whose instance
//! disappeared falls back to the default with a warning, and open windows
//! are told to refetch `GET /api/engines`.

use std::collections::HashSet;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::protocol::ServerMsg;
use crate::state::AppState;

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        if let Err(e) = watch(state).await {
            tracing::warn!(%e, "engine config watcher stopped");
        }
    });
}

async fn watch(state: AppState) -> anyhow::Result<()> {
    let path = mira_config::global_path();
    let Some(dir) = path.parent().map(|d| d.to_path_buf()) else {
        return Ok(());
    };
    let name = path.file_name().map(|n| n.to_os_string());
    let _ = std::fs::create_dir_all(&dir);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    // Watch the folder, not the file: editors save by replacing it. Only
    // events naming the config count; the same folder holds files Mira
    // rewrites constantly (state.yaml).
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                if ev
                    .paths
                    .iter()
                    .any(|p| p.file_name().map(|n| n.to_os_string()) == name)
                {
                    let _ = tx.send(());
                }
            }
        })?;
    watcher.watch(&dir, RecursiveMode::NonRecursive)?;
    while rx.recv().await.is_some() {
        tokio::time::sleep(Duration::from_millis(400)).await;
        while rx.try_recv().is_ok() {}
        reload(&state).await;
    }
    drop(watcher);
    Ok(())
}

/// Rebuild from the config on disk and reconcile everything that holds
/// engines. Safe to call any time; a config that fails to parse is ignored
/// (the previous registry stays) so a half-saved file can't break pickers.
pub async fn reload(state: &AppState) {
    let cfg = match mira_config::MiraConfig::load_global() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(%e, "mira.yaml changed but didn't parse; keeping the current engines");
            return;
        }
    };
    let next = mira_engine::EngineRegistry::from_config(&cfg);
    let native = |r: &mira_engine::EngineRegistry| -> HashSet<String> {
        r.instances()
            .filter(|i| i.is_native())
            .map(|i| i.id.to_string())
            .collect()
    };
    let before = native(&state.engines.current());
    let after = native(&next);

    let mut pools = vec![state.provider.clone()];
    for slot in state.list_slots().await {
        pools.push(slot.native_provider.clone());
    }
    for id in &after {
        match mira_engine::native::build_native_provider(&cfg, id) {
            Ok(p) => {
                for pool in &pools {
                    let active = pool.active_instance().as_deref() == Some(id.as_str());
                    pool.register(id, p.clone());
                    // A changed key applies to a chat already on this instance.
                    if active {
                        pool.activate(id);
                    }
                }
            }
            // Can't build (key removed): no row, so a switch to it reports why.
            Err(_) => pools.iter().for_each(|pool| pool.unregister(id)),
        }
    }
    for gone in before.difference(&after) {
        pools.iter().for_each(|pool| pool.unregister(gone));
    }
    for pool in &pools {
        pool.set_known_instances(after.iter().cloned());
    }
    let default = next.default_native_instance(&cfg).map(|i| i.id.to_string());
    state.engines.replace(next);

    // A chat on a removed instance moves to the default and says so.
    for slot in state.list_slots().await {
        if slot.acp_launch.lock().await.is_some() {
            continue;
        }
        let (current, _, _) = slot.selection.snapshot();
        let Some(current) = current else { continue };
        if after.contains(&current) {
            continue;
        }
        if let Some(def) = &default {
            if slot.native_provider.activate(def) {
                *slot
                    .selection
                    .instance
                    .write()
                    .expect("selection lock poisoned") = Some(def.clone());
            }
        }
        let _ = slot.events_tx.send(ServerMsg::Warning {
            text: format!(
                "Engine `{current}` was removed from mira.yaml; this chat moved to {}.",
                default.as_deref().unwrap_or("the default provider")
            ),
        });
    }
    let global = state
        .selection
        .instance
        .read()
        .expect("selection lock poisoned")
        .clone();
    if let (Some(current), Some(def)) = (global, &default) {
        if !after.contains(&current) && state.provider.activate(def) {
            *state
                .selection
                .instance
                .write()
                .expect("selection lock poisoned") = Some(def.clone());
        }
    }
    tracing::info!(instances = after.len(), "engines reloaded from mira.yaml");
    state.broadcast_all(ServerMsg::EnginesChanged).await;
}
