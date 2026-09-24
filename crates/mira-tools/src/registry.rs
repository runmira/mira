use std::collections::HashMap;
use std::sync::Arc;

use mira_ai::ToolSpec;

use crate::tool::Tool;

/// Tools whose set changes while Mira runs — the tools of connected MCP
/// servers, for example. A [`Registry`] asks each source for its current
/// tools on every lookup, so a server that connects, reconnects or goes
/// away is reflected in every session holding a clone of the registry.
pub trait ToolSource: Send + Sync {
    fn tools(&self) -> Vec<Arc<dyn Tool>>;
}

/// Ordered map of tool name → tool, plus any live [`ToolSource`]s.
///
/// Insertion order is preserved so tools appear to the model in the order
/// they were registered — useful when demonstrating a preferred toolset.
/// Source tools follow the registered ones; a registered tool wins a name
/// clash.
#[derive(Default, Clone)]
pub struct Registry {
    order: Vec<String>,
    tools: HashMap<String, Arc<dyn Tool>>,
    sources: Vec<Arc<dyn ToolSource>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool. Later registrations under the same name replace
    /// earlier ones (useful for tests to substitute a mock).
    pub fn register<T: Tool + 'static>(&mut self, tool: T) -> &mut Self {
        let arc: Arc<dyn Tool> = Arc::new(tool);
        self.register_arc(arc)
    }

    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        let name = tool.spec().name;
        if !self.tools.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.tools.insert(name, tool);
        self
    }

    /// Add a live tool source (see [`ToolSource`]).
    pub fn add_source(&mut self, source: Arc<dyn ToolSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    /// The live sources, so a derived registry (a subagent's) can keep
    /// them.
    pub fn sources(&self) -> &[Arc<dyn ToolSource>] {
        &self.sources
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if let Some(t) = self.tools.get(name) {
            return Some(t.clone());
        }
        self.source_tools()
            .into_iter()
            .find(|t| t.spec().name == name)
    }

    /// Current tools from the live sources, minus any shadowed by a
    /// registered tool or an earlier source.
    fn source_tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for source in &self.sources {
            for tool in source.tools() {
                let name = tool.spec().name;
                if !self.tools.contains_key(&name) && seen.insert(name) {
                    out.push(tool);
                }
            }
        }
        out
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools().iter().map(|t| t.spec()).collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len() + self.source_tools().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Specs of the tools usable in a remote environment (see
    /// [`Tool::remote_capable`]); what the model sees while the session
    /// is switched to one.
    pub fn remote_specs(&self) -> Vec<ToolSpec> {
        self.tools()
            .iter()
            .filter(|t| t.remote_capable())
            .map(|t| t.spec())
            .collect()
    }

    /// Names of registered tools hidden in a remote environment.
    pub fn local_only(&self) -> Vec<String> {
        self.tools()
            .iter()
            .filter(|t| !t.remote_capable())
            .map(|t| t.spec().name)
            .collect()
    }

    /// Every tool, registered ones in registration order and then the
    /// live sources' current tools. Used by callers that need to build a
    /// filtered subset (e.g. the `agent` tool constructing a child
    /// registry).
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut out: Vec<Arc<dyn Tool>> = self
            .order
            .iter()
            .filter_map(|n| self.tools.get(n).cloned())
            .collect();
        out.extend(self.source_tools());
        out
    }
}
