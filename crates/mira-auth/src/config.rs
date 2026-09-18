//! Mirror an OAuth-minted API key into `~/.mira/mira.yaml` so the
//! standard `ProviderConfig::resolved_api_key` path picks it up
//! without any special casing.
//!
//! Both the server flow (which then hot-swaps the live provider) and
//! the CLI flow (which just exits) need the same yaml write, so it
//! lives here.

use anyhow::{Context, Result};
use mira_config::{default_base_url_for, MiraConfig, ProviderConfig};

/// Write `api_key` into `providers.<provider>.api_key`, filling in a
/// sensible `base_url` when one isn't already set. Also promotes the
/// provider to `default_provider` when no default is configured, so a
/// fresh install lands on the freshly-signed-in provider without an
/// extra step.
pub fn write_provider_key(provider: &str, api_key: &str) -> Result<()> {
    let mut cfg = MiraConfig::load_global().context("load ~/.mira/mira.yaml")?;
    let entry = cfg
        .providers
        .entry(provider.to_owned())
        .or_insert_with(|| ProviderConfig {
            base_url: default_base_url_for(provider).map(str::to_owned),
            ..Default::default()
        });
    entry.api_key = Some(api_key.to_owned());
    if entry.base_url.is_none() {
        entry.base_url = default_base_url_for(provider).map(str::to_owned);
    }
    if cfg.default_provider.is_none() {
        cfg.default_provider = Some(provider.to_owned());
    }
    cfg.save_global().context("save ~/.mira/mira.yaml")?;
    Ok(())
}

/// Remove `providers.<provider>.api_key` from `~/.mira/mira.yaml`. If
/// the entry has no other fields set (base_url etc.), remove the whole
/// provider entry so `mira logout` genuinely "forgets" the provider.
/// `Ok(false)` when there was nothing to clear (idempotent).
pub fn clear_provider_key(provider: &str) -> Result<bool> {
    let mut cfg = MiraConfig::load_global().context("load ~/.mira/mira.yaml")?;
    let had_key = cfg
        .providers
        .get(provider)
        .map(|p| p.api_key.is_some())
        .unwrap_or(false);
    if !had_key {
        return Ok(false);
    }
    if let Some(entry) = cfg.providers.get_mut(provider) {
        entry.api_key = None;
        // Nothing else meaningful set? Drop the whole entry so the
        // yaml stays tidy.
        if entry.base_url == default_base_url_for(provider).map(str::to_owned)
            && entry.api_key_env.is_none()
            && entry.extra_headers.is_empty()
        {
            cfg.providers.remove(provider);
        }
    }
    if cfg.default_provider.as_deref() == Some(provider) {
        cfg.default_provider = None;
    }
    cfg.save_global().context("save ~/.mira/mira.yaml")?;
    Ok(true)
}
