//! External instances — bridging the opaque config envelope to the
//! launch-level `DriverConfig` the ACP runtime needs.
//!
//! The config blob is driver-specific and forward-safe: keys this build
//! doesn't know are ignored here but never rejected upstream, so config
//! written for a newer driver still round-trips. Strings that name
//! paths and env values pass through `shellexpand`-free verbatim —
//! expansion is the launch layer's job, not the registry's.

use mira_acp::driver::DriverConfig;
use mira_config::MiraConfig;
use serde_json::Value;

use crate::instance::EngineInstance;

/// Build a launch config from an instance's opaque blob.
///
/// Recognized keys (all optional): `binary_path`, `launch_args`,
/// `env`, `home_path`, `api_key` or `api_key_env`, `effort`, `setting_sources`,
/// `auto_compact_after`. Wrong-typed values degrade to the default
/// rather than failing the whole instance — a typo in one field should
/// not hide the other five.
pub fn driver_config_for(inst: &EngineInstance) -> DriverConfig {
    let cfg = inst.config_object();
    let str_field =
        |k: &str| -> Option<String> { cfg.get(k).and_then(Value::as_str).map(str::to_string) };
    DriverConfig {
        enabled: inst.enabled,
        display_name: inst.display_name.clone(),
        binary_path: str_field("binary_path").map(std::path::PathBuf::from),
        launch_args: cfg
            .get("launch_args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        auto_compact_after: cfg.get("auto_compact_after").and_then(Value::as_u64),
        home_path: str_field("home_path").map(std::path::PathBuf::from),
        env: cfg
            .get("env")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        // A literal key wins; `api_key_env` names a variable in Mira's own
        // environment to read it from at launch.
        api_key: str_field("api_key").filter(|k| !k.is_empty()).or_else(|| {
            str_field("api_key_env")
                .and_then(|name| std::env::var(name).ok())
                .filter(|k| !k.is_empty())
        }),
        effort: str_field("effort"),
        setting_sources: str_field("setting_sources"),
    }
}

/// Suggest the yaml key names that carry an API key for `driver`,
/// derived from the driver's own env-var convention. Used by error
/// hints: "add `config.api_key` or set ANTHROPIC_API_KEY".
pub fn api_key_hint(driver_kind: &str) -> Option<String> {
    let d = mira_acp::drivers::by_kind(driver_kind)?;
    let envs = d.api_key_env_vars();
    if envs.is_empty() {
        return None;
    }
    Some(envs.join(" / "))
}

/// The raw `engines:` entry for an instance, for callers that need the
/// yaml block verbatim (settings UI round-trip).
pub fn raw_config_from<'a>(cfg: &'a MiraConfig, instance: &str) -> Option<&'a serde_json::Value> {
    cfg.engines
        .get(instance)
        .map(|e| e.config.as_ref().unwrap_or(&serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(yaml_config: &str) -> EngineInstance {
        let full = format!(
            "default_provider: openai\nengines:\n  codex-work:\n    driver: codex\n    config: {yaml_config}\n"
        );
        let cfg: MiraConfig = serde_yaml::from_str(&full).unwrap();
        crate::instance::instances_from_config(&cfg)
            .get("codex-work")
            .unwrap()
            .clone()
    }

    #[test]
    fn known_keys_map_onto_the_launch_config() {
        let inst = instance(
            "\n      binary_path: /opt/codex-acp\n      launch_args: [\"-c\", \"x=1\"]\n      env: {CODEX_PROFILE: work}\n      api_key: sk-test\n      effort: high\n      auto_compact_after: 100000\n",
        );
        let dc = driver_config_for(&inst);
        assert_eq!(
            dc.binary_path.as_deref(),
            Some(std::path::Path::new("/opt/codex-acp"))
        );
        assert_eq!(dc.launch_args, vec!["-c".to_string(), "x=1".to_string()]);
        assert_eq!(
            dc.env.get("CODEX_PROFILE").map(String::as_str),
            Some("work")
        );
        assert_eq!(dc.api_key.as_deref(), Some("sk-test"));
        assert_eq!(dc.effort.as_deref(), Some("high"));
        assert_eq!(dc.auto_compact_after, Some(100_000));
        assert!(dc.enabled, "instance-level enabled carries through");
    }

    #[test]
    fn wrong_typed_fields_degrade_without_failing_the_rest() {
        let inst = instance(
            "\n      launch_args: \"not-a-list\"\n      env: \"neither\"\n      api_key: still-fine\n",
        );
        let dc = driver_config_for(&inst);
        assert!(dc.launch_args.is_empty());
        assert!(dc.env.is_empty());
        assert_eq!(dc.api_key.as_deref(), Some("still-fine"));
    }

    #[test]
    fn api_key_env_reads_the_named_variable_and_a_literal_key_wins() {
        // PATH is set in every test environment; it stands in for a key var.
        let path = std::env::var("PATH").unwrap();
        let dc = driver_config_for(&instance("\n      api_key_env: PATH\n"));
        assert_eq!(dc.api_key.as_deref(), Some(path.as_str()));
        let dc = driver_config_for(&instance(
            "\n      api_key: sk-literal\n      api_key_env: PATH\n",
        ));
        assert_eq!(dc.api_key.as_deref(), Some("sk-literal"));
        let dc = driver_config_for(&instance(
            "\n      api_key_env: MIRA_TEST_SURELY_UNSET_VAR\n",
        ));
        assert!(dc.api_key.is_none());
    }

    #[test]
    fn a_null_config_is_a_default_launch() {
        let inst = instance("null");
        let dc = driver_config_for(&inst);
        assert!(dc.enabled);
        assert!(dc.binary_path.is_none());
        assert!(dc.display_name.is_none());
        assert!(dc.launch_args.is_empty());
        assert!(dc.env.is_empty());
        assert!(dc.api_key.is_none());
    }
}
