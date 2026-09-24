use std::collections::HashMap;
use std::sync::Arc;

use mira_ai::ToolSpec;

use crate::tool::Tool;

/// Ordered map of tool name → tool.
///
/// Insertion order is preserved so tools appear to the model in the order
/// they were registered — useful when demonstrating a preferred toolset.
#[derive(Default, Clone)]
pub struct Registry {
    order: Vec<String>,
    tools: HashMap<String, Arc<dyn Tool>>,
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

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|n| self.tools.get(n))
            .map(|t| t.spec())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Specs of the tools usable in a remote environment (see
    /// [`Tool::remote_capable`]); what the model sees while the session
    /// is switched to one.
    pub fn remote_specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|n| self.tools.get(n))
            .filter(|t| t.remote_capable())
            .map(|t| t.spec())
            .collect()
    }

    /// Names of registered tools hidden in a remote environment.
    pub fn local_only(&self) -> Vec<String> {
        self.order
            .iter()
            .filter(|n| self.tools.get(*n).is_some_and(|t| !t.remote_capable()))
            .cloned()
            .collect()
    }

    /// Every registered tool, in registration order. Used by callers that
    /// need to build a filtered subset (e.g. the `agent` tool constructing
    /// a child registry).
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.order
            .iter()
            .filter_map(|n| self.tools.get(n).cloned())
            .collect()
    }
}
