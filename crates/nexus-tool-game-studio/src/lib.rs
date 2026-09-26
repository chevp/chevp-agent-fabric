//! Game-studio tools: the "game company made of agents" as MCP tools.
//!
//! Agents are not chatbots here, they are `goal + budget + authority + tools`
//! entries in an org chart, and they coordinate through durable state rather
//! than through conversation:
//!
//! ```text
//! projects/<id>/studio/
//! ├── org.yaml        # directors / agents, reportsTo, handles, per-job budget caps
//! ├── vision.yaml     # game vision, objectives, open problems, world facts
//! ├── jobs/JOB-*.json # the job bus: queued → claimed → done | failed
//! └── events.jsonl    # append-only event log (asset_mutated, playtest_run, ...)
//! ```
//!
//! A director reads `studio_state`, decomposes problems into jobs with
//! `studio_submit_job` (only downward in the org), agents pull work with
//! `studio_claim_job` (priority order, dependencies respected) and report
//! with `studio_complete_job`, which also records events. Every event may
//! name `parents` and a `child`, so `studio_lineage` can answer "why does
//! this rock exist?" by walking the full genealogy.
//!
//! The `studio/` directory is owned by this crate, not by `nexus-domain`:
//! the domain stays read-only and only locates the project directory. Like
//! the rest of Agent Nexus, the state is plain files meant to live in Git.
//! Blender/engine/render execution is out of scope — those are separate MCP
//! servers the agents call; this crate is the bus they report back to.

mod model;
mod studio;
mod tools;

pub use tools::{
    ClaimJobTool, CompleteJobTool, LineageTool, OrgTool, RecordEventTool, StateTool, SubmitJobTool,
};

use nexus_tools::Tool;

/// Starter `org.yaml` modelled on a research → game → world/art/gameplay/
/// playtest director hierarchy. Copy into `projects/<id>/studio/`.
pub const TEMPLATE_ORG: &str = include_str!("../templates/org.yaml");
/// Starter `vision.yaml`. Copy into `projects/<id>/studio/`.
pub const TEMPLATE_VISION: &str = include_str!("../templates/vision.yaml");

/// Every tool this crate provides, ready for `ToolRegistry::register`.
pub fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(OrgTool),
        Box::new(StateTool),
        Box::new(SubmitJobTool),
        Box::new(ClaimJobTool),
        Box::new(CompleteJobTool),
        Box::new(RecordEventTool),
        Box::new(LineageTool),
    ]
}
