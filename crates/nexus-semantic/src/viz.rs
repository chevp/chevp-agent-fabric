//! Standalone HTML explorer for a `SemanticGraph`. The graph JSON is
//! embedded, so the file works from disk without a server.

use crate::graph_view::SemanticGraph;

const TEMPLATE: &str = include_str!("../assets/semantic-project.html");
const PLACEHOLDER: &str = "/*__NEXUS_GRAPH__*/null";

pub fn render_html(graph: &SemanticGraph) -> String {
    // `</` inside a <script> block would end it early.
    let json = serde_json::to_string(graph)
        .unwrap_or_else(|_| "null".to_string())
        .replace("</", "<\\/");
    TEMPLATE.replacen(PLACEHOLDER, &json, 1)
}

/// The explorer without embedded data (loads a `semantic-graph.json` via a file picker).
pub fn template() -> &'static str {
    TEMPLATE
}
