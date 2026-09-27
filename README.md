# Agent Nexus

Agent Nexus is a Rust **Control Plane** — a central intelligence and context
layer for multiple AI clients and agents (Figma Make, GitHub Copilot, IDE
agents, and future autonomous agents). It exposes its capabilities through
the **Model Context Protocol (MCP)** over stdio, using a Git repository as
the persistent source of truth.

Agent Nexus is **not** a monolithic autonomous agent. It does not write
code, design UIs, or make product decisions. It resolves and hands out the
minimal relevant context — project knowledge, skills, behavior specs, graph
relationships, and policies — so that specialized agents can do their job
consistently.

## Design stance (read this before adding anything)

- **Filesystem/Git is the storage, full stop.** There is no `trait
  ProjectStore` / `trait GraphStore` abstraction pretending a Postgres or
  Neo4j backend might show up later. If that ever needs to change, it's a
  rewrite of the concrete types in `nexus-domain`, not a trait swap. Keeping
  that abstraction out means one less layer of indirection to read through
  for a system that, in practice, only ever has one implementation.
- **The only extension point is the `Tool` trait.** Need a concrete
  capability — run an analysis, call an external system, generate a report?
  Implement `nexus_tools::Tool` in your own crate and register it. That's
  it. There is no generic "capability registry" standing in for
  hypothetical integrations that don't exist yet.

## 1. Architecture

```
                    ┌──────────────────────────────┐
                    │        MCP Clients           │
                    │  Figma Make · Copilot ·      │
                    │  IDE Agents · Future Agents  │
                    └──────────────┬───────────────┘
                                   │  JSON-RPC 2.0 over stdio
                    ┌──────────────▼───────────────┐
                    │   crates/nexus-server        │
                    │   stdio.rs → rpc.rs          │
                    │   (hand-rolled MCP transport;│
                    │    no business logic here)   │
                    └──────────────┬───────────────┘
                                   │
              ┌────────────────────┼───────────────────────┐
              ▼                                            ▼
   ┌────────────────────┐                     ┌─────────────────────────┐
   │  domain_tools.rs   │                     │      ToolRegistry       │
   │  (fixed read API:  │                     │  crates/nexus-tools     │
   │  projects, skills, │                     │                         │
   │  behavior, graph,  │                     │  Tool trait — the ONLY  │
   │  policies, context)│                     │  extension point        │
   └──────────┬─────────┘                     └─────────────┬───────────┘
              │                                             │
              ▼                                             ▼
   ┌─────────────────────────────────────┐     ┌──────────────────────────────────┐
   │        crates/nexus-domain          │     │ crates/nexus-tool-graph-insights │
   │  Project/Skill/Behavior/Graph/Policy│◄────│ (example concrete tool crate:    │
   │  stores + the Context Resolver      │     │  runs a relation-count analysis) │
   │  — filesystem/Git-backed, concrete  │     └──────────────────────────────────┘
   └──────────────────┬──────────────────┘
                      ▼
              ┌────────────────────┐
              │   Git / Filesystem │
              │  projects/, global/│
              └────────────────────┘
```

Every crate depends only downward. `nexus-domain` knows nothing about MCP or
tools. `nexus-tools` knows the domain (read-only) but nothing about the
transport. `nexus-server` wires domain + tool registry to the wire protocol
and contains no domain logic of its own — `rpc.rs` and `domain_tools.rs`
only parse/dispatch.

## 2. Repository structure

```
agent-nexus/ (workspace root)
├── Cargo.toml                        # workspace manifest
├── crates/
│   ├── nexus-domain/                 # projects, skills, behavior, graph, policies, context resolver
│   │   └── src/{types,errors,fs_util,project,skill,behavior,graph,policy,context}.rs
│   ├── nexus-tools/                  # the Tool trait + ToolRegistry (the extension point)
│   ├── nexus-tool-graph-insights/    # example concrete tool crate (a graph analysis)
│   └── nexus-server/                 # binary: domain_tools.rs, rpc.rs, stdio.rs, app.rs, main.rs
│
├── projects/
│   └── acme-app/                     # example project
│       ├── project.yaml
│       ├── context/
│       ├── skills/
│       ├── behavior/
│       ├── graph/{entities,relations}/
│       └── policies/
│
├── global/
│   ├── skills/                       # skills shared by every project
│   └── policies/                     # default policies shared by every project
│
├── schemas/                          # JSON Schema mirrors of the domain types (language-agnostic docs)
└── README.md
```

## 3. Project model

A **Project** is the bounded workspace and source of truth for a body of
work, declared by `project.yaml` at the root of its directory:

```yaml
id: acme-app
name: Acme App
version: 1
description: Example e-commerce application...
```

The directory name must match `id`. `ProjectStore` (`crates/nexus-domain/src/project.rs`)
discovers any directory under `projects/` that has a `project.yaml` — no
registration step.

## 4. Skill model

A **Skill** is a Markdown file with `---`-delimited YAML frontmatter:

```markdown
---
id: checkout-ux
name: Checkout UX
version: 1.0.0
scope: project
description: Rules for implementing checkout interactions
tags: [checkout, ux]
dependsOn: [accessibility]
---

# Checkout UX
... agent instructions in Markdown ...
```

- `scope` must match where the file lives (`global/skills/...` vs.
  `projects/<id>/skills/...`); a mismatch is a validation error.
- A project skill **overrides** a global skill declared under the same
  `id` (`SkillStore::list_skills`).
- `dependsOn` is resolved transitively (`skill::resolve_skills`).
- `resolve_skills(..., task, ...)` additionally matches skills by simple
  keyword overlap between the task text and a skill's id/name/description/
  tags — this is what lets the Context Resolver find skills without the
  caller knowing their ids up front.

## 5. Behavior model

A **Behavior Specification** is structured YAML — not Markdown — so the
same spec can be consumed by a design tool, a frontend agent, a backend
agent, a test-generation agent, or a QA agent:

```yaml
id: checkout-button
type: ui-behavior
version: 1
states: [idle, loading, disabled, success, error]
transitions:
  - { from: idle, event: submit, to: loading }
  - { from: loading, event: success, to: success }
  - { from: loading, event: failure, to: error }
rules:
  - id: prevent-double-submit
    description: User cannot submit twice while loading
  - id: preserve-input
    description: User input must remain available after an error
```

Every transition's `from`/`to` must reference a declared state; this is
validated on load (`BehaviorStore::list_behaviors`), not deferred to the
consumer.

## 6. Semantic Content Graph

Entities and relations are plain files under `graph/entities/` and
`graph/relations/`:

```yaml
# graph/entities/checkout-button.yaml
id: checkout-button
type: component
name: Checkout Button
```

```yaml
# graph/relations/checkout-button-governed-by-checkout-ux.yaml
from: checkout-button
relation: governed-by
to: checkout-ux
```

Supported relation kinds: `implements`, `depends-on`, `uses`, `defined-by`,
`governed-by`, `represented-by`, `implemented-by`, `validated-by`,
`related-to`.

`GraphStore` (`crates/nexus-domain/src/graph.rs`) exposes `list_entities`,
`list_relations`, `get_entity`; query helpers on top: `get_related_entities`
(bounded BFS), `find_path` (shortest path by hop count), `search_graph`
(substring search). This is a plain in-memory traversal over parsed YAML,
not a graph database — see the design stance above for why there's no
`Neo4jGraphStore`-shaped abstraction waiting for one.

## 7. Context resolution

`context::resolve_context` (`crates/nexus-domain/src/context.rs`) is the
core of the Control Plane. Given `{ projectId, task, client, requestedEntities? }`,
it:

1. Loads the project and checks the client has `read:project`.
2. Resolves relevant graph entities (`requestedEntities`, or a keyword
   search over the task text) and expands one hop.
3. Resolves skills relevant to the task, plus any skill that
   `governs`/`implements` a resolved entity, plus their `dependsOn` closure.
4. Resolves behavior specs whose id matches a resolved entity or the task.
5. Resolves the client's policy rules for the project.
6. Returns a single, minimal `ResolvedContext` — never the whole project.

```
Task: "Implement the checkout button"
         │
         ▼
  Context Resolver
         │
   ┌─────┼─────────────┬─────────────┐
   ▼     ▼             ▼             ▼
Project Skills:      Behavior:     Graph:
Acme App checkout-ux, checkout-    checkout-button,
         accessibility button      payment-flow,
                                   checkout-summary
         │
         ▼
   Policies: whatever the requesting client is granted
         │
         ▼
   Resolved Context (returned via the resolve_context MCP tool)
```

## 8. MCP interface

Agent Nexus speaks MCP's JSON-RPC 2.0 methods directly over stdio
(newline-delimited messages) — `crates/nexus-server/src/stdio.rs` reads
lines, `rpc.rs` dispatches `initialize`, `tools/list`, `tools/call`, `ping`.
There is no MCP SDK dependency; the protocol surface used here is small
enough to own.

Fixed read tools (`domain_tools.rs`, always present):

| Tool | Description |
| --- | --- |
| `list_projects` | Lists all projects. |
| `get_project` | Gets a project by id. |
| `list_skills` | Lists skills, optionally scoped to a project. |
| `get_skill` | Gets a skill by id, optionally scoped to a project. |
| `resolve_skills` | Resolves skills relevant to a task, with dependency closure. |
| `list_behaviors` | Lists a project's behavior specs. |
| `get_behavior` | Gets one behavior spec. |
| `get_entity` | Gets one graph entity. |
| `search_graph` | Searches graph entities. |
| `get_related_entities` | BFS traversal from an entity. |
| `get_project_policy` | Gets a client's policy rules for a project. |
| `check_permission` | Checks whether a client holds a permission. |
| `resolve_context` | Resolves the minimal relevant context for a task. |

Plus every tool registered in `ToolRegistry`:

| Tool | Description |
| --- | --- |
| `graph_insights` | Example concrete tool: ranks a project's graph entities by relation count. |
| `studio_org` | Game studio org chart (directors → agents) with structural problems. |
| `studio_state` | Director's view: vision, problems, job queue, ready jobs, spend, recent events. |
| `studio_submit_job` | Puts a job on the bus; authority flows downward, budget within the assignee's cap. |
| `studio_claim_job` | An agent pulls its highest-priority ready job (exclusive claim). |
| `studio_complete_job` | Reports done/failed with result, spend (overruns flagged) and events. |
| `studio_record_event` | Appends events (`asset_mutated`, ...) with parents/child to the log. |
| `studio_lineage` | Walks the genealogy of an artifact back to founders or forward to leaves. |

`tools/list` returns both sets, indistinguishable to the client. Domain
errors and tool errors alike come back as `{ content: [...], isError: true }`
— they never crash the process or the JSON-RPC connection.

## 9. Adding a concrete tool (the extension point)

This is the one designed extension seam. To add "a tool MCP calls to run an
analysis":

1. `cargo new --lib crates/nexus-tool-<name>` and add it as a workspace
   member.
2. Implement `nexus_tools::Tool`:

   ```rust
   pub struct MyTool;
   impl Tool for MyTool {
       fn name(&self) -> &str { "my_tool" }
       fn description(&self) -> &str { "..." }
       fn input_schema(&self) -> Value { json!({ "type": "object", ... }) }
       fn call(&self, ctx: &ToolContext<'_>, input: Value) -> Result<Value, ToolError> {
           // read via ctx.domain.{projects,skills,behaviors,graph,policies}
           // and return whatever analysis result makes sense as JSON.
       }
   }
   ```
3. In `crates/nexus-server/src/app.rs::build_tool_registry`, add
   `registry.register(Box::new(nexus_tool_my_name::MyTool));` and add the
   crate as a dependency of `nexus-server`.

Nothing else changes: no transport code, no domain code. See
`crates/nexus-tool-graph-insights` for a complete, working example.

### Game studio tools (`crates/nexus-tool-game-studio`)

A multi-agent game studio coordinated through files instead of chat. Agents
are `goal + handled job types + tools + budget cap + authority` entries in
an org chart; they never talk to each other directly, they read state, pull
jobs and report results. Blender/engine/render execution stays in separate
MCP servers the agents call — this crate is the bus they report back to.

```
projects/<id>/studio/
├── org.yaml         # members: id, role (director|agent), reportsTo, handles, tools, budget
├── vision.yaml      # vision, objectives, problems, facts
├── jobs/JOB-*.json  # queued → claimed → done | failed   (written by the tools)
└── events.jsonl     # append-only event log            (written by the tools)
```

The loop: the game director reads `studio_state` and turns problems into
jobs (`studio_submit_job`, only downward in the org, budget within the
assignee's per-job cap, `dependsOn` for ordering). Agents call
`studio_claim_job` — they get jobs assigned to themselves, or to their lead
when the type is in their `handles`. `studio_complete_job` records result,
spend and events such as `{ event: asset_mutated, parent: rock_0042, child:
rock_0042_17, operator: fracture_crystal, seed: 38192, scores: {...} }`, and
`studio_lineage` answers "why does this rock exist?" from those events.

Start a project from `crates/nexus-tool-game-studio/templates/{org,vision}.yaml`.
This crate is the one place that writes files: it owns `studio/`, and
`nexus-domain` stays read-only.

## 10. Permission model

Policies are plain files (`<client>.yaml`) under `global/policies/` and
`projects/<id>/policies/`:

```yaml
client: copilot
permissions:
  - read:project
  - read:skills
  - read:behavior
  - read:graph
  - write:code-reference
```

`PolicyEngine` (`crates/nexus-domain/src/policy.rs`) is independent of MCP
— a plain domain service. For a given `(client, project)`, project-scoped
rules for that client **replace** (not merge with) its global rules; a
client with no project-specific rule falls back to its global rule.
Clients (`copilot`, `figma-make`, an IDE agent, ...) are never hard-coded
into the domain model — they are just ids that happen to appear in policy
files.

## 11. How to add a new project

```bash
mkdir -p projects/<id>/{context,skills,behavior,graph/entities,graph/relations,policies}
```

Add `projects/<id>/project.yaml` with `id`, `name`, `version`. Everything
else is optional; a project with none of skills/behavior/graph/policies is
valid and simply resolves to global skills/policies only.

## 12. How to add a new skill

1. Decide scope: reusable → `global/skills/<id>/skill.md`; project-specific
   → `projects/<id>/skills/<id>/skill.md`.
2. Write frontmatter (`id`, `name`, `version`, `scope`, `description`,
   optional `tags`/`dependsOn`) matching that location.
3. Write the instructions as the Markdown body.
4. To override a global skill for one project, give it the same `id` under
   that project's `skills/` directory.

## 13. Installing the MCP server in a project

Agent Nexus ships as a single compiled binary (`agent-nexus`) — there is no
npm package, so installation is either "build from source" or "download the
release binary". Releases are cut by pushing a `vX.Y.Z` tag, which triggers
[`.github/workflows/release.yml`](.github/workflows/release.yml) and
publishes a GitHub Release with prebuilt binaries for Linux, macOS
(x86_64 + aarch64) and Windows.

**Option A — download a release binary**

```bash
# Pick the asset for your platform from the latest release:
# https://github.com/chevp/chevp-agent-fabric/releases/latest
curl -L -o agent-nexus \
  https://github.com/chevp/chevp-agent-fabric/releases/latest/download/agent-nexus-linux-x86_64
chmod +x agent-nexus
```

**Option B — build from source**

```bash
git clone https://github.com/chevp/chevp-agent-fabric.git
cd chevp-agent-fabric
cargo build --release
# binary at ./target/release/agent-nexus
```

**Register it with an MCP client** (Claude Code example):

```bash
claude mcp add agent-nexus \
  --env AGENT_NEXUS_ROOT=/path/to/your/project \
  -- /path/to/agent-nexus
```

`AGENT_NEXUS_ROOT` points at the Git repository that acts as the Control
Plane's source of truth (projects/, global/) and defaults to the process's
current working directory if unset. Any MCP client that speaks JSON-RPC 2.0
over stdio can be pointed at the binary the same way. Quick manual check:

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | ./target/release/agent-nexus
```

### Updating

There is no self-updating daemon — Agent Nexus is a local stdio process, not
a long-running service, so "updating" means picking up a newer released
binary and restarting the MCP client's connection to it:

1. Watch [Releases](https://github.com/chevp/chevp-agent-fabric/releases)
   (or `git tag --list 'v*'`) for a newer version than the one reported by
   the running server's `initialize` response (`serverInfo.version`, which
   is `env!("CARGO_PKG_VERSION")` — always in sync with the tag it was built
   from).
2. Replace the binary in place — re-run the curl download for the new
   release asset (Option A), or `git pull && cargo build --release` at the
   new tag (Option B). The binary path stays the same, so no MCP client
   config change is needed.
3. Restart the MCP client (or just the `agent-nexus` process) so it
   reconnects to the new binary.

A new version is cut by a maintainer pushing a tag:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

`release.yml` then builds and publishes the binaries automatically; nothing
else needs to change (the version comes from the tag name, not from a
checked-in file).

## 14. Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all
```

### Example project

`projects/acme-app` is a runnable example (used by the test suite) that
demonstrates every layer: a `checkout-ux` project skill depending on the
global `accessibility` skill, a `checkout-button` behavior spec, three graph
entities (`checkout-button`, `payment-flow`, `checkout-summary`) tied
together by `governed-by`/`uses`/`related-to`/`depends-on` relations, and
`copilot`/`figma-make` policies.

Tests live next to the code they cover (`#[cfg(test)] mod tests` in each
domain module) plus `crates/nexus-server/src/rpc.rs`, which walks the exact
scenario from the architecture doc — *"Create the checkout flow according
to our current UX rules"* — through the real JSON-RPC dispatcher, including
a call to the registered `graph_insights` tool.
