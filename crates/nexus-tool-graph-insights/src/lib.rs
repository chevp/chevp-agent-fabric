//! Example concrete tool: `graph_insights`.
//!
//! This is what "a tool called by MCP that runs an analysis" looks like in
//! practice. It does not touch the domain's storage internals directly — it
//! only reads through `NexusDomain`'s public API (`domain.graph`) — and it
//! adds nothing to the domain crate itself. A real analysis tool (linting a
//! design system, computing a dependency-cycle check, scoring test
//! coverage, ...) follows the exact same shape: implement `Tool` in your own
//! crate, register it in `nexus-server`.

use nexus_domain::graph::{get_related_entities, RelatedEntitiesOptions};
use nexus_tools::{Tool, ToolContext, ToolError};
use serde::Deserialize;
use serde_json::{json, Value};

pub struct GraphInsightsTool;

#[derive(Deserialize)]
struct Input {
    #[serde(rename = "projectId")]
    project_id: String,
}

#[derive(serde::Serialize)]
struct EntityInsight {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    name: String,
    #[serde(rename = "relationCount")]
    relation_count: usize,
}

impl Tool for GraphInsightsTool {
    fn name(&self) -> &str {
        "graph_insights"
    }

    fn description(&self) -> &str {
        "Analyzes a project's semantic graph and ranks entities by how many relations touch them (a simple centrality measure), useful for spotting the most load-bearing components."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["projectId"],
            "additionalProperties": false,
            "properties": {
                "projectId": { "type": "string", "minLength": 1 }
            }
        })
    }

    fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
        let input: Input = serde_json::from_value(input)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?;

        let entities = ctx.domain.graph.list_entities(&input.project_id)?;
        let mut insights: Vec<EntityInsight> = Vec::with_capacity(entities.len());
        for entity in &entities {
            let (_, relations) = get_related_entities(
                &ctx.domain.graph,
                &input.project_id,
                &entity.id,
                RelatedEntitiesOptions {
                    depth: 1,
                    ..Default::default()
                },
            )?;
            insights.push(EntityInsight {
                id: entity.id.clone(),
                kind: entity.kind.clone(),
                name: entity.name.clone(),
                relation_count: relations.len(),
            });
        }
        insights.sort_by(|a, b| {
            b.relation_count
                .cmp(&a.relation_count)
                .then_with(|| a.id.cmp(&b.id))
        });

        Ok(json!({ "projectId": input.project_id, "entities": insights }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_domain::NexusDomain;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn ranks_checkout_button_as_the_most_connected_entity() {
        let domain = NexusDomain::from_repo_root(repo_root());
        let ctx = ToolContext { domain: &domain };
        let tool = GraphInsightsTool;

        let result = tool.call(&ctx, json!({ "projectId": "acme-app" })).unwrap();
        let entities = result["entities"].as_array().unwrap();

        assert_eq!(entities[0]["id"], "checkout-button");
        assert!(entities[0]["relationCount"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn rejects_missing_project_id() {
        let domain = NexusDomain::from_repo_root(repo_root());
        let ctx = ToolContext { domain: &domain };
        let tool = GraphInsightsTool;

        assert!(matches!(
            tool.call(&ctx, json!({})),
            Err(ToolError::InvalidInput(_))
        ));
    }
}
