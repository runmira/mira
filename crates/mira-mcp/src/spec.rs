//! What an MCP server is: where its definition came from (scope) and how
//! to reach it (transport). Also the naming rules that turn server and
//! tool names into model-safe tool names, and `${VAR}` expansion.

use std::collections::BTreeMap;
use std::path::PathBuf;

use mira_config::{
    McpHttpConfig, McpOAuthConfig, McpRemoteKind, McpServerConfig, McpStdioConfig, McpStdioKind,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Where a server definition lives. Precedence for servers sharing a
/// name, highest first: local, project, user. Plugin servers get their
/// own names (`plugin_<plugin>_<server>`) and never clash.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// `mcp_servers:` in `~/.mira/mira.yaml`: every project.
    User,
    /// `.mcp.json` (or `mcp_servers:` in `.mira/config.yaml`) in the
    /// project: shared through the repo, so it needs approval before it
    /// runs.
    Project,
    /// This project only, private to you (kept in `~/.mira/mcp/state.json`).
    Local,
    /// Shipped by an installed plugin.
    Plugin { plugin: String },
}

impl Scope {
    pub fn label(&self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::Project => "project",
            Scope::Local => "local",
            Scope::Plugin { .. } => "plugin",
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Scope::Local => 3,
            Scope::Project => 2,
            Scope::User => 1,
            Scope::Plugin { .. } => 0,
        }
    }

    /// True when `self` wins a name clash against `other`.
    pub fn outranks(&self, other: &Scope) -> bool {
        self.rank() > other.rank()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        cwd: Option<String>,
    },
    /// Streamable HTTP.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        oauth: Option<McpOAuthConfig>,
    },
    /// The older HTTP+SSE transport.
    Sse {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        oauth: Option<McpOAuthConfig>,
    },
}

impl Transport {
    pub fn kind(&self) -> &'static str {
        match self {
            Transport::Stdio { .. } => "stdio",
            Transport::Http { .. } => "http",
            Transport::Sse { .. } => "sse",
        }
    }

    pub fn url(&self) -> Option<&str> {
        match self {
            Transport::Http { url, .. } | Transport::Sse { url, .. } => Some(url),
            Transport::Stdio { .. } => None,
        }
    }

    /// One line for lists: the command line, or the URL.
    pub fn summary(&self) -> String {
        match self {
            Transport::Stdio { command, args, .. } => {
                let mut s = command.clone();
                for a in args {
                    s.push(' ');
                    s.push_str(a);
                }
                s
            }
            Transport::Http { url, .. } | Transport::Sse { url, .. } => url.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSpec {
    /// Unique among the servers Mira runs; also the key for status,
    /// reconnects and permission rules.
    pub name: String,
    pub scope: Scope,
    pub transport: Transport,
    /// The file the definition came from, for display.
    #[serde(default)]
    pub source: Option<PathBuf>,
    /// For plugin servers: the plugin's directory, exposed to the
    /// definition as `${CLAUDE_PLUGIN_ROOT}` / `${MIRA_PLUGIN_ROOT}`.
    #[serde(default)]
    pub plugin_root: Option<PathBuf>,
}

impl ServerSpec {
    pub fn from_config(name: &str, scope: Scope, cfg: &McpServerConfig) -> Self {
        Self {
            name: name.to_owned(),
            scope,
            transport: transport_from_config(cfg),
            source: None,
            plugin_root: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<PathBuf>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Stable key for per-server settings (enabled/disabled).
    pub fn id(&self) -> String {
        format!("{}:{}", self.scope.label(), self.name)
    }

    /// Hash of what the server would run or reach. A project server is
    /// approved against this, so editing its command asks again.
    pub fn fingerprint(&self) -> String {
        let json = serde_json::to_vec(&self.transport).unwrap_or_default();
        let digest = Sha256::digest(&json);
        digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
    }

    pub fn is_remote(&self) -> bool {
        !matches!(self.transport, Transport::Stdio { .. })
    }

    /// Variables available to `${...}` expansion besides the process
    /// environment.
    pub fn extra_vars(&self) -> BTreeMap<String, String> {
        let mut vars = BTreeMap::new();
        if let Some(root) = &self.plugin_root {
            let root = root.display().to_string();
            vars.insert("CLAUDE_PLUGIN_ROOT".to_owned(), root.clone());
            vars.insert("MIRA_PLUGIN_ROOT".to_owned(), root);
        }
        vars
    }

    pub fn to_config(&self) -> McpServerConfig {
        match &self.transport {
            Transport::Stdio {
                command,
                args,
                env,
                cwd,
            } => McpServerConfig::Stdio(McpStdioConfig {
                kind: None,
                command: command.clone(),
                args: args.clone(),
                env: env.clone(),
                cwd: cwd.clone(),
            }),
            Transport::Http {
                url,
                headers,
                oauth,
            } => McpServerConfig::Http(McpHttpConfig {
                kind: None,
                url: url.clone(),
                headers: headers.clone(),
                auth: None,
                oauth: oauth.clone(),
            }),
            Transport::Sse {
                url,
                headers,
                oauth,
            } => McpServerConfig::Http(McpHttpConfig {
                kind: Some(McpRemoteKind::Sse),
                url: url.clone(),
                headers: headers.clone(),
                auth: None,
                oauth: oauth.clone(),
            }),
        }
    }
}

pub fn transport_from_config(cfg: &McpServerConfig) -> Transport {
    match cfg {
        McpServerConfig::Stdio(s) => Transport::Stdio {
            command: s.command.clone(),
            args: s.args.clone(),
            env: s.env.clone(),
            cwd: s.cwd.clone(),
        },
        McpServerConfig::Http(h) => {
            let mut headers = h.headers.clone();
            if let Some(auth) = &h.auth {
                headers
                    .entry("Authorization".to_owned())
                    .or_insert_with(|| auth.clone());
            }
            match h.kind.unwrap_or_default() {
                McpRemoteKind::Http => Transport::Http {
                    url: h.url.clone(),
                    headers,
                    oauth: h.oauth.clone(),
                },
                McpRemoteKind::Sse => Transport::Sse {
                    url: h.url.clone(),
                    headers,
                    oauth: h.oauth.clone(),
                },
            }
        }
    }
}

/// Parse Claude Code's `.mcp.json` shape — `{"mcpServers": {name: {...}}}`
/// — or a bare `{name: {...}}` map. Lenient on purpose: unknown fields
/// are ignored, so a file written for another client still loads. Each
/// entry that can't be understood comes back as an error string instead
/// of failing the whole file.
pub fn parse_mcp_json(
    text: &str,
) -> Result<Vec<(String, Result<McpServerConfig, String>)>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let map = root
        .get("mcpServers")
        .or_else(|| root.get("mcp_servers"))
        .unwrap_or(&root);
    let Some(map) = map.as_object() else {
        return Err("expected an object of servers".into());
    };
    Ok(map
        .iter()
        .map(|(name, v)| (name.clone(), config_from_json(v)))
        .collect())
}

/// One server entry in `.mcp.json` form.
pub fn config_from_json(v: &Value) -> Result<McpServerConfig, String> {
    let Some(obj) = v.as_object() else {
        return Err("expected an object".into());
    };
    let str_map = |key: &str| -> BTreeMap<String, String> {
        obj.get(key)
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| match v {
                        Value::String(s) => Some((k.clone(), s.clone())),
                        Value::Number(n) => Some((k.clone(), n.to_string())),
                        Value::Bool(b) => Some((k.clone(), b.to_string())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let kind = obj.get("type").and_then(Value::as_str).unwrap_or("");
    if let Some(url) = obj.get("url").and_then(Value::as_str) {
        let kind = match kind {
            "sse" => McpRemoteKind::Sse,
            "" | "http" | "streamable-http" | "streamable_http" | "streamableHttp" => {
                McpRemoteKind::Http
            }
            other => return Err(format!("unknown transport type `{other}`")),
        };
        let oauth = obj
            .get("oauth")
            .and_then(|o| serde_json::from_value::<McpOAuthConfig>(o.clone()).ok());
        return Ok(McpServerConfig::Http(McpHttpConfig {
            kind: Some(kind),
            url: url.to_owned(),
            headers: str_map("headers"),
            auth: None,
            oauth,
        }));
    }
    let Some(command) = obj.get("command").and_then(Value::as_str) else {
        return Err("needs `command` (stdio) or `url` (http/sse)".into());
    };
    if !matches!(kind, "" | "stdio") {
        return Err(format!("`type: {kind}` needs a `url`"));
    }
    let args = obj
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerConfig::Stdio(McpStdioConfig {
        kind: Some(McpStdioKind::Stdio),
        command: command.to_owned(),
        args,
        env: str_map("env"),
        cwd: obj.get("cwd").and_then(Value::as_str).map(str::to_owned),
    }))
}

/// Expand `${VAR}` and `${VAR:-default}` against `extra`, then the
/// process environment. A variable that's unset and has no default is
/// an error naming it: connecting with a literal `${GITHUB_TOKEN}` would
/// only fail later and less clearly.
pub fn expand(s: &str, extra: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let inner = &after[..end];
        let (var, default) = match inner.split_once(":-") {
            Some((v, d)) => (v, Some(d)),
            None => (inner, None),
        };
        let value = extra
            .get(var)
            .cloned()
            .or_else(|| std::env::var(var).ok().filter(|v| !v.is_empty()))
            .or_else(|| default.map(str::to_owned));
        match value {
            Some(v) => out.push_str(&v),
            None => return Err(var.to_owned()),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// `${VAR}`s in `s` with no value in `extra` or the environment and no
/// default.
pub fn missing_vars(s: &str, extra: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else { break };
        let inner = &after[..end];
        if !inner.contains(":-") {
            let set =
                extra.contains_key(inner) || std::env::var(inner).is_ok_and(|v| !v.is_empty());
            if !set && !out.iter().any(|o| o == inner) {
                out.push(inner.to_owned());
            }
        }
        rest = &after[end + 1..];
    }
    out
}

/// Turn a server or tool name into the character set tool names allow
/// (`[A-Za-z0-9_-]`). `__` is reserved as the separator in
/// `mcp__<server>__<tool>`, so runs of underscores collapse to one.
pub fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_ascii_alphanumeric() || c == '-' {
            c
        } else {
            '_'
        };
        if c == '_' && out.ends_with('_') {
            continue;
        }
        out.push(c);
    }
    let out = out.trim_matches('_').to_owned();
    if out.is_empty() {
        "server".to_owned()
    } else {
        out
    }
}

/// Tool names are capped at 64 characters by the model APIs.
const MAX_TOOL_NAME: usize = 64;

/// The name the model sees: `mcp__<server>__<tool>`, shortened with a
/// hash suffix when it would pass 64 characters.
pub fn tool_name(server: &str, tool: &str) -> String {
    let server = sanitize(server);
    // Tool names keep their own underscores; only the separator matters.
    let tool: String = tool
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let full = format!("mcp__{server}__{tool}");
    if full.len() <= MAX_TOOL_NAME {
        return full;
    }
    let digest = Sha256::digest(full.as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    let keep = MAX_TOOL_NAME - hash.len() - 1;
    format!("{}_{hash}", &full[..keep])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_vars_and_defaults() {
        let mut extra = BTreeMap::new();
        extra.insert("ROOT".to_owned(), "/p".to_owned());
        assert_eq!(expand("${ROOT}/bin", &extra).unwrap(), "/p/bin");
        assert_eq!(
            expand("${MIRA_TEST_UNSET_VAR:-fallback}", &extra).unwrap(),
            "fallback"
        );
        assert_eq!(
            expand("a ${MIRA_TEST_UNSET_VAR} b", &extra).unwrap_err(),
            "MIRA_TEST_UNSET_VAR"
        );
        assert_eq!(expand("no vars $HOME", &extra).unwrap(), "no vars $HOME");
    }

    #[test]
    fn names_are_model_safe() {
        assert_eq!(sanitize("my server.v2"), "my_server_v2");
        assert_eq!(sanitize("a__b"), "a_b");
        assert_eq!(sanitize("..."), "server");
        assert_eq!(
            tool_name("github", "create_issue"),
            "mcp__github__create_issue"
        );
        let long = tool_name("a-very-long-server-name-indeed", &"x".repeat(60));
        assert_eq!(long.len(), 64);
        assert!(long.starts_with("mcp__a-very-long-server-name-indeed__x"));
        assert_ne!(
            long,
            tool_name("a-very-long-server-name-indeed", &"x".repeat(61))
        );
    }

    #[test]
    fn reads_claude_code_mcp_json() {
        let text = r#"{
          "mcpServers": {
            "gh": {"type": "http", "url": "https://api.example.com/mcp", "headers": {"X-Key": "${K}"}},
            "legacy": {"type": "sse", "url": "https://x.dev/sse"},
            "fs": {"command": "npx", "args": ["-y", "server-fs", 3], "env": {"A": "1"}, "timeout": 5},
            "bad": {"type": "ws"}
          }
        }"#;
        let parsed = parse_mcp_json(text).unwrap();
        let get = |n: &str| parsed.iter().find(|(k, _)| k == n).unwrap().1.clone();
        let gh = transport_from_config(&get("gh").unwrap());
        assert!(matches!(gh, Transport::Http { ref headers, .. } if headers["X-Key"] == "${K}"));
        assert_eq!(transport_from_config(&get("legacy").unwrap()).kind(), "sse");
        let fs = transport_from_config(&get("fs").unwrap());
        assert!(
            matches!(fs, Transport::Stdio { ref args, .. } if args == &["-y", "server-fs", "3"])
        );
        assert!(get("bad").is_err());
    }

    #[test]
    fn config_roundtrip_and_fingerprint() {
        let spec = ServerSpec {
            name: "x".into(),
            scope: Scope::Project,
            transport: Transport::Sse {
                url: "https://x.dev/sse".into(),
                headers: BTreeMap::new(),
                oauth: None,
            },
            source: None,
            plugin_root: None,
        };
        let back = ServerSpec::from_config("x", Scope::Project, &spec.to_config());
        assert_eq!(back.transport, spec.transport);
        let mut other = spec.clone();
        other.transport = Transport::Sse {
            url: "https://evil.dev/sse".into(),
            headers: BTreeMap::new(),
            oauth: None,
        };
        assert_ne!(spec.fingerprint(), other.fingerprint());
    }
}
