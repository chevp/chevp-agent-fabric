# Agent Nexus

Agent Nexus is a production-oriented Node.js/TypeScript **Control Plane** — a
central intelligence and context layer for multiple AI clients and agents
(Figma Make, GitHub Copilot, IDE agents, and future autonomous agents). It
exposes its capabilities through the **Model Context Protocol (MCP)**, using
a Git repository as the initial persistent source of truth.

Agent Nexus is **not** a monolithic autonomous agent. It does not write code,
design UIs, or make product decisions. It resolves and hands out the minimal
relevant context — project knowledge, skills, behavior specs, graph
relationships, and policies — so that specialized agents can do their job
consistently.

---

## 1. Architecture

```
                         ┌────────────────────────────┐
                         │        MCP Clients          │
                         │  Figma Make · Copilot ·      │
                         │  IDE Agents · Future Agents  │
                         └──────────────┬───────────────┘
                                        │  MCP (stdio / HTTP)
                         ┌──────────────▼───────────────┐
                         │        MCP Transport          │
                         │  src/server/mcp-server.ts      │
                         │  src/server/transport.ts       │
                         └──────────────┬───────────────┘
                                        │  calls into (no business logic here)
                         ┌──────────────▼───────────────┐
                         │     Application Services       │
                         │   src/app/nexus-services.ts     │
                         └──────────────┬───────────────┘
                                        │
        ┌──────────────┬───────────────┼───────────────┬──────────────┐
        ▼              ▼               ▼               ▼              ▼
   ┌─────────┐   ┌───────────┐   ┌───────────┐   ┌───────────┐  ┌───────────┐
   │ Context │   │  Skills   │   │ Behavior  │   │   Graph   │  │ Policies  │
   │Resolver │   │ Registry/ │   │  Store/   │   │  Store/   │  │  Engine   │
   │         │   │ Resolver  │   │ Resolver  │   │  Query/   │  │           │
   │         │   │           │   │           │   │ Resolver  │  │           │
   └────┬────┘   └─────┬─────┘   └─────┬─────┘   └─────┬─────┘  └─────┬─────┘
        │              │               │               │              │
        └──────────────┴───────┬───────┴───────────────┴──────────────┘
                                ▼
                     ┌────────────────────┐
                     │   Domain Model      │
                     │  src/shared/types    │
                     └──────────┬──────────┘
                                ▼
                     ┌────────────────────┐
                     │       Stores        │
                     │ ProjectStore ·       │
                     │ SkillStore ·         │
                     │ BehaviorStore ·      │
                     │ GraphStore ·         │
                     │ PolicyStore          │
                     │ (interfaces)         │
                     └──────────┬──────────┘
                                ▼
                     ┌────────────────────┐
                     │   Git / Filesystem   │
                     │  projects/, global/  │
                     └────────────────────┘
```

Layers are strictly one-directional: **MCP Transport → Application Services
→ Domain Model → Stores → Git/Filesystem**. No MCP tool handler contains
business logic — every handler parses its input and calls a method on
`NexusServices`.

Every store is defined as a TypeScript interface (`ProjectStore`,
`SkillStore`, `BehaviorStore`, `GraphStore`, `PolicyStore`). V1 ships exactly
one implementation of each, backed by the filesystem/Git (`Fs*Store`). A
future database-backed implementation (Postgres, Neo4j, a vector store, ...)
can be dropped in without touching the Context Resolver, the MCP server, or
any other domain/application code — see [§13](#13-future-migration-path).

## 2. Repository structure

```
agent-nexus/
├── src/
│   ├── app/               # Application services (composition root; MCP calls this)
│   ├── server/             # MCP server + transports (stdio, debug HTTP)
│   ├── projects/           # Project loading/registry
│   ├── skills/             # Skill loading, registry, resolution
│   ├── context/            # Context docs loader + the Context Resolver
│   ├── graph/              # Semantic Content Graph store/query/resolver
│   ├── behavior/           # Behavior specification store/resolver
│   ├── policies/           # Policy engine + permission checks
│   ├── capabilities/       # External system/tool abstraction
│   ├── workflow/           # Level-4 workflow interfaces (types only, unused in V1)
│   └── shared/             # Domain types, zod schemas, errors, fs helpers
│
├── projects/
│   └── acme-app/           # Example project (see below)
│       ├── project.yaml
│       ├── context/
│       ├── skills/
│       ├── behavior/
│       ├── graph/{entities,relations}/
│       └── policies/
│
├── global/
│   ├── skills/             # Skills shared by every project
│   └── policies/           # Default policies shared by every project
│
├── schemas/                # JSON Schema mirrors of the zod validators
├── tests/                  # Vitest suite (unit + end-to-end)
├── package.json
├── tsconfig.json
└── README.md
```

## 3. Project model

A **Project** is the bounded workspace and source of truth for a body of
work. It is declared by a `project.yaml` at the root of its directory:

```yaml
id: acme-app
name: Acme App
version: 1
description: Example e-commerce application...
```

The directory name must match `id`. Project-specific skills, behavior specs,
graph data, and policies live under that same directory
(`projects/acme-app/...`); anything reusable across projects lives under
`global/...`.

## 4. Skill model

A **Skill** is a Markdown file with machine-readable YAML frontmatter:

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
  `id` (`SkillRegistry.listSkills`).
- `dependsOn` is resolved transitively (`resolveSkills`): asking for
  `checkout-ux` also returns `accessibility`.
- `resolveSkills({ task })` additionally matches skills by simple keyword
  overlap between the task text and a skill's id/name/description/tags —
  this is what lets the Context Resolver find skills without the caller
  knowing their ids up front.

## 5. Behavior model

A **Behavior Specification** is a structured (YAML), technology-agnostic
state machine — not Markdown — so the same spec can be consumed by a design
tool, a frontend agent, a backend agent, a test-generation agent, or a QA
agent:

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
validated on load (`FsBehaviorStore`), not deferred to the consumer.

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

`GraphStore` is a clean abstraction (`getEntity`, `listEntities`,
`listRelations`) with query helpers on top (`graph-query.ts`):
`getRelatedEntities` (bounded BFS), `findPath` (shortest path by hop count),
`searchGraph` (substring search). This is deliberately not a graph database —
a real one (Neo4j, etc.) can implement `GraphStore` later.

## 7. Context resolution

The **Context Resolver** (`src/context/context-resolver.ts`) is the core of
the Control Plane. Given a `{ projectId, task, client, requestedEntities? }`
request, it:

1. Loads the project and checks the client has `read:project`.
2. Resolves the relevant graph entities (`requestedEntities`, or a keyword
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
   Resolved Context (returned via resolve_context)
```

## 8. MCP interface

Read tools (V1 ships read-only; see [§9](#9-permission-model) for how writes
will be gated once added):

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

Every tool: parses its (zod-validated) input, calls exactly one
`NexusServices` method, and returns JSON in the tool's text content; domain
errors (`NotFoundError`, `PermissionDeniedError`, ...) are returned as
`isError: true` tool results instead of crashing the server.

Planned write operations (**not implemented in V1**, gated by the Policy
Engine when they land): `create_entity`, `update_behavior`, `create_skill`,
`link_entities`, `record_decision`.

## 9. Permission model

Policies are plain files (`client.yaml`) under `global/policies/` and
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

`PolicyEngine` (`src/policies/policy-engine.ts`) is independent of MCP — it
is a plain domain service. For a given `(client, project)`, project-scoped
rules for that client **replace** its global rules (they don't merge); a
client with no project-specific rule falls back to its global rule. Clients
(`copilot`, `figma-make`, an IDE agent, ...) are never hard-coded into the
domain model — they are just ids that happen to appear in policy files.

## 10. How to add a new project

1. `mkdir -p projects/<id>/{context,skills,behavior,graph/entities,graph/relations,policies}`
2. Add `projects/<id>/project.yaml` with `id`, `name`, `version`.
3. Add any project-specific skills/behavior/graph/policies as needed — a
   project with none of these is valid; it will simply resolve empty
   context beyond global skills/policies.
4. Commit. There is no registration step: `FsProjectStore` discovers any
   directory under `projects/` that has a `project.yaml`.

## 11. How to add a new skill

1. Decide the scope: reusable across projects → `global/skills/<id>/skill.md`;
   project-specific → `projects/<id>/skills/<id>/skill.md`.
2. Write the frontmatter (`id`, `name`, `version`, `scope`, `description`,
   optional `tags`/`dependsOn`) matching that location — a mismatch fails
   validation.
3. Write the instructions as the Markdown body.
4. If the skill should override a same-named global skill for one project,
   just give it the same `id` under that project's `skills/` directory.

## 12. How to connect an MCP client

```bash
npm install
npm run build
node dist/index.js         # stdio MCP server (default)
```

Point any MCP-compatible client (Claude, an IDE agent, a custom harness) at
this process over stdio. For local debugging without an MCP client, a
minimal HTTP surface is available:

```bash
AGENT_NEXUS_TRANSPORT=http PORT=3333 node dist/index.js
curl localhost:3333/projects
curl -X POST localhost:3333/resolve-context \
  -H 'content-type: application/json' \
  -d '{"projectId":"acme-app","task":"Implement the checkout button","client":{"id":"copilot","type":"coding-agent"}}'
```

The HTTP surface is a debugging convenience, not a second implementation of
MCP negotiation — it calls the exact same `NexusServices` methods as the MCP
tools.

## 13. Future migration path

Every store is behind an interface (`ProjectStore`, `SkillStore`,
`BehaviorStore`, `GraphStore`, `PolicyStore`) that only the `Fs*` classes in
`src/app/nexus-services.ts` know about. Introducing a database means:

1. Implement the interface against the new backend (e.g. `PostgresProjectStore`,
   `Neo4jGraphStore`).
2. Swap the construction in `NexusServices` (or make it configurable).
3. Nothing in `src/context`, `src/server`, or any resolver changes, because
   they only depend on the interfaces, never on "files" or "YAML".

Git-backed storage stays valuable even after a database is introduced — it's
the review/version/rollback/collaboration layer for the *declarative*
project data (skills, behavior specs, graph fixtures) that a database would
otherwise need to reinvent.

## 14. Future work: Level-4 workflows

`src/workflow/types.ts` declares (but does not implement) `Workflow`,
`WorkflowState`, `Task`, `TaskExecution`, `AgentRun`, `Artifact`, `Decision`,
and `Verification`. Nothing else in the codebase depends on them today. They
exist so that a future orchestration layer — e.g.

```
UX Definition -> Behavior Specification -> Figma -> Implementation
  -> Tests -> Verification -> Human Approval
```

— has a stable shape to grow into, without V1 committing to a workflow
engine, a scheduler, or multi-agent planning.

## 15. Development

```bash
npm install
npm run dev          # run the MCP server over stdio via tsx
npm test             # vitest — unit + end-to-end tests
npm run lint
npm run format:check
npm run build         # tsc -> dist/
```

### Example project

`projects/acme-app` is a runnable example (used by the test suite) that
demonstrates every layer: a `checkout-ux` project skill depending on the
global `accessibility` skill, a `checkout-button` behavior spec, three graph
entities (`checkout-button`, `payment-flow`, `checkout-summary`) tied
together by `governed-by`/`uses`/`related-to`/`depends-on` relations, and
`copilot`/`figma-make` policies.

`tests/e2e.test.ts` walks the full scenario from the architecture doc —
*"Create the checkout flow according to our current UX rules"* — through
`resolve_context` over an in-memory MCP transport, then double-checks the
resulting permission grant via `check_permission`.
