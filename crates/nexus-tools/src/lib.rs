//! The `Tool` trait is the *only* extension point of Agent Nexus.
//!
//! There is no plugin system for swapping storage backends, and no
//! "capability" abstraction standing in for a future graph database. If a
//! concrete piece of tooling is needed — running an analysis, calling out to
//! an external system, generating a report — it is implemented as a `Tool`
//! in its own crate and registered with a `ToolRegistry`. The MCP server
//! exposes every registered tool exactly like its built-in domain tools.

use nexus_domain::NexusDomain;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("{0}")]
    Domain(#[from] nexus_domain::NexusError),
    #[error("{0}")]
    Other(String),
}

/// Read-only access to the domain services, handed to a `Tool` when it runs.
/// A tool never mutates domain state directly (there are no write stores in
/// V1) — it reads project/graph/skill/behavior data and computes something
/// from it.
pub struct ToolContext<'a> {
    pub domain: &'a NexusDomain,
}

/// A concrete, MCP-callable capability. Implement this trait in your own
/// crate to add a new tool; nothing in `nexus-server` needs to change beyond
/// registering an instance.
pub trait Tool: Send + Sync {
    /// Stable, unique tool name as it appears over MCP (`tools/list`, `tools/call`).
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema (draft-07-ish object schema) describing the tool's input.
    fn input_schema(&self) -> Value;
    /// Runs the tool. `input` has already been checked against `input_schema`
    /// as far as JSON structure goes; deeper validation is the tool's job.
    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError>;
}

/// Descriptor returned to MCP clients via `tools/list`.
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Holds every registered `Tool`, keyed by name. Registration order does not
/// matter; names must be unique.
#[derive(Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn list(&self) -> Vec<ToolDescriptor> {
        self.tools
            .values()
            .map(|t| ToolDescriptor {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn call(
        &self,
        ctx: &ToolContext<'_>,
        name: &str,
        input: Value,
    ) -> Result<Value, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::InvalidInput(format!("unknown tool \"{name}\"")))?;
        tool.call(ctx, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct EchoTool;
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes its input back."
        }
        fn input_schema(&self) -> Value {
            json!({ "type": "object" })
        }
        fn call(&self, _ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
            Ok(input)
        }
    }

    #[test]
    fn registers_and_lists_a_tool() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));

        let descriptors = registry.list();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].name, "echo");
    }

    #[test]
    fn calling_an_unknown_tool_is_an_error() {
        let registry = ToolRegistry::new();
        let domain = NexusDomain::from_repo_root(".");
        let ctx = ToolContext { domain: &domain };

        assert!(matches!(
            registry.call(&ctx, "nope", json!({})),
            Err(ToolError::InvalidInput(_))
        ));
    }
}
