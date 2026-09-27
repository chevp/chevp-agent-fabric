//! `index_project`: scans a project's real source tree and writes matching
//! Semantic Content Graph entities/relations and imported skill files under
//! its Nexus directory.
//!
//! Scope (icc-frost-lib-shaped projects):
//! - `icc-frost-lib/labs/<aN>-<slug>` -> one `graph/entities/<aN>.yaml`
//!   (type: lab), enriched with `class`/`derivedFrom` from `labs/INDEX.md`
//!   when present, plus a `depends-on` relation to the lab it derives from.
//! - `icc-frost-lib/moana/kits.json` -> one `graph/entities/moana-<name>.yaml`
//!   (type: moana-kit) per kit, plus a `uses` relation to every lab it lists
//!   as a member.
//! - `.claude/skills/<id>/SKILL.md` -> one `skills/<id>/skill.md`, since that
//!   frontmatter (`name`, `description`) is not a valid Nexus skill on its
//!   own (missing `id`/`version`/`scope`).
//!
//! Every generated file carries a `generated-by: nexus-tool-indexer` marker
//! so re-running is safe: a file with the marker is refreshed in place, a
//! file without it (hand-authored, same path) is left untouched and reported
//! as skipped. `dryRun: true` previews without writing. This crate is the
//! one place that writes `graph/` and project `skills/` files for a project
//! it indexes — `nexus-domain` stays read-only.

mod scan;
mod tool;
mod write;

pub use tool::IndexProjectTool;

use nexus_tools::Tool;

/// Every tool this crate provides, ready for `ToolRegistry::register`.
pub fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(IndexProjectTool)]
}
