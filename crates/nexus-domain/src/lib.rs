//! Agent Nexus domain model.
//!
//! This crate is the whole Control Plane brain: it loads projects, skills,
//! behavior specifications and the semantic graph from a Git-backed
//! filesystem layout, evaluates policies, and resolves the minimal relevant
//! context for a task. It has no knowledge of MCP, HTTP, or any transport.
//!
//! Storage here is concrete and filesystem-only. There is deliberately no
//! generic "pluggable backend" abstraction to prepare for a future database
//! (Neo4j, Postgres, ...) — Git/the filesystem *is* the source of truth for
//! V1, full stop. If that ever needs to change, it is a rewrite of the
//! `*Store` types in this crate, not a trait swap.
//!
//! The only intended extension point of the whole system is concrete
//! tooling: see the `nexus-tools` crate for the `Tool` trait that
//! additional crates implement to add MCP-callable analyses.

pub mod behavior;
pub mod context;
pub mod errors;
pub mod fs_util;
pub mod graph;
pub mod policy;
pub mod project;
pub mod skill;
pub mod types;

#[cfg(test)]
mod tests_support;

pub use errors::{NexusError, NexusResult};

use behavior::BehaviorStore;
use graph::GraphStore;
use policy::{PolicyEngine, PolicyStore};
use project::ProjectStore;
use skill::SkillStore;
use std::path::{Path, PathBuf};

/// Wires together every store for one repository root
/// (`<root>/projects`, `<root>/global`).
pub struct NexusDomain {
    pub projects_root: PathBuf,
    pub global_root: PathBuf,
    pub projects: ProjectStore,
    pub skills: SkillStore,
    pub behaviors: BehaviorStore,
    pub graph: GraphStore,
    pub policies: PolicyEngine,
}

impl NexusDomain {
    pub fn from_repo_root(repo_root: impl AsRef<Path>) -> Self {
        let repo_root = repo_root.as_ref();
        let projects_root = repo_root.join("projects");
        let global_root = repo_root.join("global");
        Self {
            projects: ProjectStore::new(&projects_root),
            skills: SkillStore::new(&global_root, &projects_root),
            behaviors: BehaviorStore::new(&projects_root),
            graph: GraphStore::new(&projects_root),
            policies: PolicyEngine::new(PolicyStore::new(&global_root, &projects_root)),
            projects_root,
            global_root,
        }
    }
}
