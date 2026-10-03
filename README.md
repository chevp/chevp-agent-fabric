# Agent Nexus

Agent Nexus is a Rust **Control Plane** — a central intelligence and context
layer for multiple AI clients and agents (Figma Make, GitHub Copilot, IDE
agents, and future autonomous agents). It exposes its capabilities through
the **Model Context Protocol (MCP)** over stdio, using a Git repository as
the persistent source of truth.

Agent Nexus is a **semantic translation layer** between specialized agents.
It turns project artifacts from Git — design specs, HTML prototypes, design
tokens, behavior specs, SKILL.md files, code, contracts, commits — into one
intermediate representation, the `SemanticModel`, and translates, validates,
diffs and consolidates that model into a reviewable **semantic graph**. It
also resolves and hands out the minimal relevant context for a task.

Agent Nexus is **not** a monolithic autonomous agent. It does not write
code, design UIs, make product or architecture decisions, or overwrite
project truth on its own: new or inferred knowledge only enters the
canonical graph through a proposal a human (or a permitted client) accepts.

## Design stance (read this before adding anything)

- **Filesystem/Git is the storage, full stop.** There is no `trait
  ProjectStore` / `trait GraphStore` abstraction pretending a Postgres or
  Neo4j backend might show up later. If that ever needs to change, it's a
  rewrite of the concrete types in `nexus-domain`, not a trait swap. Keeping
  that abstraction out means one less layer of indirection to read through
  for a system that, in practice, only ever has one implementation.
- **Two kinds of extension points, on two levels.** `nexus_tools::Tool` is
  the *MCP* extension point: a concrete capability (an analysis, an external
  system, a report) implemented in its own crate and registered. Inside the
  semantic domain, `ArtifactParser`, `Translator` and `Validator`
  (`crates/nexus-semantic`) are the *domain* extension points. This
  replaces the earlier rule "the only extension point is `Tool`", which
  could not express per-format parsing or per-role translation without
  putting semantics into MCP handlers. There is still no generic
  "capability registry".
- **Meaning, not files.** Nothing is translated file-to-file. Every
  artifact is parsed into a `SemanticModel`; every target representation is
  produced from a `SemanticModel`.
- **No silent facts.** Every statement carries evidence (`explicit`,
  `inferred`, `candidate`, `unknown`), a confidence and its provenance.

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
         ┌─────────────────────────┼──────────────────────────────┐
         ▼                         ▼                              ▼
┌──────────────────┐   ┌───────────────────────┐   ┌──────────────────────────┐
│ domain_tools.rs  │   │ semantic_tools.rs     │   │ ToolRegistry             │
│ (fixed read API) │   │ resources.rs (nexus://│   │ crates/nexus-tools       │
│                  │   │ thin adapters only)   │   │ Tool = MCP extension pt. │
└────────┬─────────┘   └───────────┬───────────┘   └────────────┬─────────────┘
         │                         ▼                            │
         │           ┌─────────────────────────────┐            │
         │           │ crates/nexus-semantic       │            │
         │           │ SemanticEngine (service)    │            │
         │           │  Parser → SemanticModel     │            │
         │           │  Translator · Validator     │            │
         │           │  diff · Proposal · ingest   │            │
         │           │  graph view · explorer      │            │
         │           └──────────────┬──────────────┘            │
         ▼                          ▼                           ▼
┌──────────────────────────────────────────────┐   ┌──────────────────────────┐
│ crates/nexus-domain                          │◄──│ tool crates (graph-      │
│ Project/Skill/Behavior/Graph/Policy stores   │   │ insights, game-studio,   │
│ + Context Resolver — filesystem/Git-backed   │   │ indexer, subserver)      │
└──────────────────────┬───────────────────────┘   └──────────────────────────┘
                       ▼
          ┌──────────────────────────────┐
          │ Git / Filesystem             │
          │ projects/, global/           │
          │ projects/<id>/semantic/      │
          └──────────────────────────────┘
```

Every crate depends only downward. `nexus-domain` knows nothing about MCP or
tools. `nexus-semantic` builds on the domain and knows nothing about MCP.
`nexus-tools` knows the domain (read-only) but nothing about the transport.
`nexus-server` wires domain, semantic engine and tool registry to the wire
protocol and contains no domain logic of its own — `rpc.rs`,
`domain_tools.rs`, `semantic_tools.rs` and `resources.rs` only
parse/dispatch.

## 2. Repository structure

```
agent-nexus/ (workspace root)
├── Cargo.toml                        # workspace manifest
├── crates/
│   ├── nexus-domain/                 # projects, skills, behavior, graph, policies, context resolver
│   │   └── src/{types,errors,fs_util,project,skill,behavior,graph,policy,context}.rs
│   ├── nexus-semantic/               # semantic translation layer (see section 8)
│   │   ├── src/{model,confidence,engineering,contract,consolidate,validate,translate,diff,proposal,ingest,git,graph_view,store,viz,engine}.rs
│   │   ├── src/parser/               # ArtifactParser + one parser per format
│   │   └── assets/semantic-project.html
│   ├── nexus-tools/                  # the Tool trait + ToolRegistry (MCP extension point)
│   ├── nexus-tool-graph-insights/    # example concrete tool crate (a graph analysis)
│   └── nexus-server/                 # binary: domain_tools.rs, semantic_tools.rs, resources.rs, rpc.rs, stdio.rs, app.rs, main.rs
│
├── projects/
│   └── acme-app/                     # example project
│       ├── project.yaml
│       ├── context/
│       ├── skills/
│       ├── behavior/
│       ├── graph/{entities,relations}/   # canonical graph (written only by accepted proposals or by hand)
│       ├── contracts/                    # semantic contracts
│       ├── design/                       # HTML design spec, tokens, component specs
│       ├── policies/
│       └── semantic/                     # generated: artifacts, models, translations, proposals, exports
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
roles: [product, ux, design, engineering, qa]   # optional, semantic roles
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
`related-to`, `tested-by`, `changed-by`, `contains`, `has-variant`,
`documents`.

Entities and relations may additionally carry `evidence`
(`explicit` | `inferred` | `candidate` | `unknown`, default `explicit`),
`confidence` (`{ value, reason }`), `provenance` (list of sources) and — for
entries written by an accepted proposal — `proposal`. Entities may name the
registered `artifact` they represent. Hand-written files without these
fields load exactly as before.

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

## 8. Semantic translation layer (`crates/nexus-semantic`)

```
Git / files ─▶ Acquire (register_artifact, ingest_repository)
            ─▶ Parse / extract (ArtifactParser, deterministic)
            ─▶ SemanticModel ─▶ Validate ─▶ Translate ─▶ Diff ─▶ Proposal
            ─▶ Review (human / permitted client) ─▶ canonical graph/ ─▶ semantic graph view
```

The Context Resolver (section 7) stays, as a later step: the
`resolve_semantic_context` tool returns its result plus the
EngineeringContext of the resolved skills and the model statements about
the resolved entities.

### Artifacts, kinds, roles

`NexusArtifact { id, project_id, kind, role, name, content, provenance,
confidence, metadata }`. `content` is a repo-relative file path or inline
text. Kinds: `project`, `requirement`, `design-spec`, `behavior-spec`,
`component`, `design-system`, `skill`, `policy`, `code`, `api`,
`data-model`, `test`, `documentation`, `workflow`, `commit`, `file`,
`contract`, `directory`. Roles: `product`, `ux`, `design`, `engineering`,
`qa`, `architecture`, `agent`, `policy`, `security`, `system`, or a custom
string. This `Role` is independent of the game-studio org roles
(`director`/`agent`).

### SemanticModel

`intent`, `requirements`, `behaviors` (transitions `from --event--> to` on a
subject), `entities`, `states`, `constraints` (`must`, `must-not`,
`convention`, `accessibility`), `interactions`, `dependencies` (typed
relations), `assumptions`, plus the model-level `confidence` and
`provenance`. Every statement has `evidence`, `confidence` and
`provenance`.

### What is extracted deterministically (`explicit`)

| Parser | Source | Explicit statements |
| --- | --- | --- |
| `BehaviorSpecParser` | `behavior/*.yaml` | states, transitions, rules (as constraints), declared `intent` |
| `SkillParser` | `skill.md` | frontmatter entity + `dependsOn`; sections `Purpose`, `Capabilities`, `Constraints`/`Rules`, `Conventions`, `Forbidden`, `Inputs`, `Outputs`, `Examples` → `EngineeringContext` (other sections kept under `other`) |
| `ContractParser` | `contracts/*.yaml` | intent, requirements, states, transitions, constraints on the contract subject |
| `DesignSystemParser` | tokens (`$value`/`value` leaves), component specs | tokens, components, variants, states, token usage, interactions, accessibility rules, docs/tests links |
| `HtmlParser` | `*.html` | buttons/fields/forms/links, `data-component`, `data-state(s)`, `disabled`, `required`, `aria-*`, submit/click/navigate interactions, form containment |
| `CodeMetadataParser` | `rs ts tsx js jsx py go java …` | imports, declared symbols, tests (`#[test]`, `test_*`, `describe/it/test`), variant unions |
| `MarkdownParser` | `*.md` | items under recognized headings (same headings as skills) |
| `YamlParser` / `JsonParser` | other YAML/JSON | recognized keys only (`intent`, `requirements`, `constraints`, `states`, `dependsOn`, `dependencies`), project/policy/workflow/OpenAPI/package.json shapes |
| `CommitParser` | `git log` | commit entity, `file changed-by commit` |
| `FallbackParser` | anything else | nothing; records an `unknown` assumption so the gap is visible |

### What is inferred (and by which rule)

| Statement | Evidence | Rule |
| --- | --- | --- |
| `subject defined-by behavior:<id>` | inferred | behavior spec id equals entity id (the Context Resolver's convention) |
| interaction from a behavior event | inferred | event name is in the user-action lexicon (`submit`, `click`, `reset`, …) |
| constraint from prose outside recognized headings | inferred | modal keyword (`must`, `never`, `do not`, …) |
| component from code | inferred | PascalCase function/const in a JSX file |
| HTTP interaction from code | inferred | string literal passed to `fetch`/`.get`/`.post`/… |
| `entity changed-by commit` | inferred | entity is defined in a file that commit touched |
| identifiers in backticks | candidate | mention only |
| intent from a button label | candidate | guess from label text |
| LLM suggestions (`LlmTranslator`) | candidate | never replace deterministic output |

### Confidence

Deterministic, from evidence and the number of distinct sources
(`src/confidence.rs`):

| evidence | sources | value |
| --- | --- | --- |
| explicit | any | 1.0 |
| inferred | 1 / 2 / ≥3 | 0.5 / 0.7 / 0.8 (cap) |
| candidate | 1 | 0.3 |
| unknown | – | 0.0 |

When models are consolidated, statements with the same id merge: explicit
wins if any source is explicit; a candidate seen in two independent sources
becomes inferred; nothing becomes explicit without an explicit source. The
model confidence is the mean of its statements, with the counts per
evidence level in `reason`. The validator flags inconsistencies (inferred
> 0.8, candidate > 0.5, explicit < 1.0, inferred without a reason).

### Provenance

Every statement lists all its sources: `{ source, lineStart, lineEnd,
commit, extraction, artifact }`. `source` is repo-relative (forward
slashes), `git:<sha>` for commits, `inline:<name>` for inline content,
`llm:<provider>` for LLM suggestions. Moving text around changes line
numbers but is not a semantic change.

### Validation

`missing_id`, `duplicate_id`, `missing_field`, `invalid_transition`,
`unknown_entity`, `invalid_reference`, `inconsistent_relation`,
`conflicting_constraints` (must vs. must-not on the same subject),
`missing_provenance`, `confidence_mismatch`; for translations also
`untraceable_output` and `unresolved_conflict`; against a contract
`contract_state_missing` and `contract_requirement_unverified`.

### Translation

`TranslationDirection`: `design-to-engineering`, `engineering-to-design`,
`product-to-engineering`, `behavior-to-engineering`,
`engineering-to-behavior`, `skill-to-engineering`. The
`DeterministicTranslator` only regroups existing statements (each output
item keeps its evidence and provenance) into an `EngineeringSpec`
(EngineeringContext, components, state machines, acceptance checks), a
`BehaviorRepresentation` (same YAML shape as `behavior/*.yaml`) or a
`DesignRepresentation`. With `contextSkills`, target-system rules are
attached and clashes are reported as
`{ type: constraint_conflict, resolution: requires_human_decision }` —
never resolved by Nexus. `LlmTranslator<P: LlmProvider>` is the hook for a
model-backed translator; it only adds `candidate` items. The traits are
`async`; the server drives them with `nexus_semantic::block_on`.

### Semantic diff

Statement-level: `IntentAdded`, `StateRemoved`, `BehaviorChanged`,
`ConstraintChanged`, `DependencyChanged` (relation kind of the same pair
changed), … with `{ type, category, id, entity, change }`. Diffs run against
the stored model, another artifact, or a Git revision of the same file;
ingestion diffs the files of the newest commit against its parent.

### Proposals

```
extract ─▶ SemanticModel ─▶ validate ─▶ diff against canonical graph ─▶ Proposal (pending)
        ─▶ review_proposal: accept ─▶ graph/entities, graph/relations (evidence unchanged)
                            reject ─▶ nothing written
```

A proposal lists `add-entity` / `add-relation` changes with their basis.
Structure (files, directories, modules) and authored sources (skills,
behaviors, contracts, policies — already canonical as files) are not
proposed as entities. A newer pending proposal of the same scope marks
older ones `superseded`. Accepting requires the `review:proposals`
permission (example: `projects/acme-app/policies/human-reviewer.yaml`) and
writes the statements **with their original evidence**: an inferred
relation becomes canonical *as inferred* — never as explicit.

### Semantic contracts

`projects/<id>/contracts/*.yaml` describe what must stay stable across roles:

```yaml
id: checkout-submit
role: ux
subject: checkout-button
intent: [submit checkout order]
requirements: [prevent duplicate submission, error is recoverable]
behaviors:
  states: [idle, loading, success, error]
constraints: [preserve user input on error]
```

`validate` with `contractId` checks a model against it.
`nexus://projects/<id>` lists the project's roles and contracts
(`NexusProject`).

### Semantic Knowledge Layer for Agents

A canonical, versioned, queryable vocabulary of project terms — extending
`SemanticContract` above with a term/alias identity, a kind taxonomy, a
structured definition and content/dependency freshness
(`crates/nexus-semantic/src/knowledge.rs`). This is not a second store, graph
or proposal workflow: entries are still `contracts/*.yaml`, still resolved
through `NexusProject`, and still canonicalized through the same
`propose`/`review_proposal` lifecycle as any other proposal.

```yaml
id: checkout-submit
term: CheckoutSubmit
aliases: [checkout submit, submit checkout]
kind: behavior
definition:
  short: Submits the current checkout order for processing.
  semantic: The state machine and constraints a checkout submission must satisfy...
# id, role, subject, intent, requirements, behaviors, constraints: as above
confidence: { value: 1.0 }   # defaults to explicit for a hand-authored entry
identity:                     # optional; set by whatever computed the hashes
  contentSha256: "..."
  semanticSha256: "..."
  semanticSchemaVersion: "1"
  dependencyFingerprint: "..."
```

`kind` is a closed vocabulary (`concept`, `component`, `behavior`,
`requirement`, `interaction`, `policy`, `skill`, `design-token`, `api`,
`data-model`, `workflow`, `actor`, `domain`, `feature`, `artifact`) plus a
`Custom` escape hatch — same `From<String>` pattern as `Role`, never a
project-specific enum variant.

MCP tools (`crates/nexus-server/src/knowledge_tools.rs`):

| Tool | Behavior |
| --- | --- |
| `get_knowledge_entry` | One entry by exact id. |
| `list_knowledge_entries` | Every entry in a project. |
| `search_knowledge` | Substring over id/term/display name/definition/aliases. |
| `resolve_term` | Free text → canonical entry: exact id, then exact term, then an alias (case-insensitive). |
| `get_term_relations` | Delegates to `nexus_domain::graph::get_related_entities` on the entry's `subject` — no separate graph. |
| `get_term_provenance` | The entry's source-file provenance. |
| `get_term_freshness` | Re-reads the entry's source file and its subject's current graph relations, compares against `identity`: `fresh` / `stale-content` / `stale-dependency` / `stale-schema` / `unknown` (no `identity` recorded). |
| `propose_knowledge_entry` | Builds a candidate entry from direct input (term, kind, definition, ...) and submits it as a `Pending` proposal. Reviewed with the same `review_proposal` tool as any other proposal — accepting writes `contracts/<id>.yaml`. |

Extraction is deliberately manual, not automatic: a human or an agent calls
`inspect` first, reads the resulting `SemanticModel`, and distills what
belongs in the entry before calling `propose_knowledge_entry`. Nothing here
guesses a term out of arbitrary text.

The broader `SemanticEngine` MCP surface above (`register_artifact`,
`translate`, `diff`, `ingest_repository`, `resolve_semantic_context`,
`export_semantic_graph`) is unrelated to this layer and works unchanged.

### Git ingestion

`ingest_repository` (or `agent-nexus ingest <projectId> [dir]`) walks the
project directory (skipping `.git`, `target`, `node_modules`, …, and the
project's own `semantic/`, `graph/`, `studio/`), classifies files by path
convention, registers and parses every recognized file, adds the project's
global skills, records the last N commits as commit artifacts, builds the
directory structure (`dir:` / `file:` + `contains`), derives `changed-by`
edges, computes semantic diffs of the newest commit, validates, and creates
a pending proposal. Everything is stored under `projects/<id>/semantic/`;
re-ingesting replaces what the previous run produced.

### Semantic graph and explorer

`nexus://projects/<id>/graph` combines the canonical graph, authored
sources and the model. Each node has `status` (evidence), `canonical`
(in `graph/` or an authored source), `confidence`, `provenance`, `cluster`
(`skills`, `policies`, `tests`, `behaviors`, `contracts`, `design`,
`features`, `domains`, `apis`, `code`, `docs`, `history`, …), `depth` /
`distance` from the project root, `parent`, `children` and degree
`centrality` — plain data, not tied to a renderer. `export_semantic_graph`
(or `agent-nexus export-graph <projectId>`) writes `semantic-graph.json`
and `semantic-project.html`: a standalone radial "semantic flower" explorer
(clusters as petals, zoom/pan, search, entity/relation/status filters,
details with provenance, semantic diff, `#<node-id>` deep links; no server,
no external libraries).

## 9. MCP interface

Agent Nexus speaks MCP's JSON-RPC 2.0 methods directly over stdio
(newline-delimited messages) — `crates/nexus-server/src/stdio.rs` reads
lines, `rpc.rs` dispatches `initialize`, `tools/list`, `tools/call`,
`resources/list`, `resources/templates/list`, `resources/read`, `ping`.
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

Semantic tools (`semantic_tools.rs`, thin adapters over `SemanticEngine`):

| Tool | Description |
| --- | --- |
| `register_artifact` | Registers a repo file (`path`) or inline `content` as a NexusArtifact with provenance. |
| `inspect` | Parses an artifact into a SemanticModel and stores it. |
| `translate` | Translates artifacts' (or the project's) model into a target role; optional `contextSkills`. |
| `validate` | Validates artifacts, a translation or the project; optional `contractId`. |
| `diff` | Semantic diff against the stored model, another artifact, or a Git `revision`. |
| `propose` | Creates a pending proposal for the canonical graph. |
| `review_proposal` | Accepts or rejects a proposal (`review:proposals` permission). |
| `ingest_repository` | Full repository + Git history ingestion (directory must be inside the repo root). |
| `list_artifacts` | Registered artifacts of a project. |
| `resolve_semantic_context` | `resolve_context` + EngineeringContext + model statements. |
| `export_semantic_graph` | Writes `semantic-graph.json` and `semantic-project.html`. |

Resources (read-only context; tools are for actions):

| URI | Content |
| --- | --- |
| `nexus://projects/{id}` | NexusProject (roles, semantic contracts), artifact and proposal counts |
| `nexus://projects/{id}/semantic` | Consolidated SemanticModel (`stored`, or `live` if nothing was ingested) |
| `nexus://projects/{id}/skills` | Skills with their EngineeringContext |
| `nexus://projects/{id}/behaviors` | Behavior specs |
| `nexus://projects/{id}/graph` | Semantic graph with layout metrics |
| `nexus://projects/{id}/policies` | Global and project policy rules |
| `nexus://projects/{id}/proposals[/{proposalId}]` | Proposals |
| `nexus://projects/{id}/artifacts[/{artifactId}]` | Artifacts (+ stored model) |
| `nexus://projects/{id}/translations/{translationId}` | A stored translation |

Unknown URIs return JSON-RPC error `-32002`. Reading a resource never writes.

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

## 10. Adding a concrete tool (the MCP extension point)

This is the MCP-level extension seam. To add "a tool MCP calls to run an
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

To teach the semantic layer a new format, implement
`nexus_semantic::parser::ArtifactParser` (`supports` + `inspect` returning a
`SemanticModel`) and register it with
`SemanticEngine::parsers_mut().register(...)`; new target representations
implement `nexus_semantic::translate::Translator`, new checks
`nexus_semantic::validate::Validator`.

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

### Sub-servers (`crates/nexus-tool-subserver`, `tooling/*`)

Heavier or domain-specific tooling ships as its **own MCP server binary**
under `tooling/`, runnable and testable on its own, and Agent Nexus mounts
it. At startup `agent-nexus` reads `<AGENT_NEXUS_ROOT>/subservers.json` (or
the path in `AGENT_NEXUS_SUBSERVERS`), spawns each enabled server over stdio,
calls `tools/list` once and registers one proxy `Tool` per remote tool.
`tools/call` is forwarded; a crashed child is restarted on the next call, a
server that does not start is logged and skipped. Sub-server tools never
shadow built-in or in-process tools (use `prefix` on a name clash).

```json
{ "servers": [ {
    "name": "asset-factory",
    "command": "C:/chevp/apps/kosmos/tools/chevp-agent-fabric/target/release/asset-factory-mcp.exe",
    "env": { "ASSET_FACTORY_ROOT": "G:/ft/icc-frost-lib/labs/a56-asset-factory.lab" },
    "prefix": "", "timeoutSecs": 120, "enabled": true
} ] }
```

`tooling/asset-factory-mcp` is the first one: an asset factory for
AI-generated 3D assets (design spec → multi-view references → Hunyuan3D 2.1
shape per component → Blender assembly → PBR per material group →
validation → comparison card). It plans and reviews; image, Hunyuan and
Blender execution are drivers that read its plans (see icc-frost-lib lab
`a56-asset-factory.lab`). Tools: `factory_materials`, `factory_list_specs`,
`factory_validate_spec`, `factory_reference_prompts`,
`factory_material_groups`, `factory_plan` (levels `exploration` | `hero`),
`factory_card_rig`, `factory_review` (regenerates only failing components,
mirrors map to their source, escalates after 4 attempts).

## 11. Permission model

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
files. `review:proposals` is the permission that lets a client accept or
reject semantic proposals.

## 12. How to add a new project

```bash
mkdir -p projects/<id>/{context,skills,behavior,graph/entities,graph/relations,policies}
```

Add `projects/<id>/project.yaml` with `id`, `name`, `version`. Everything
else is optional; a project with none of skills/behavior/graph/policies is
valid and simply resolves to global skills/policies only.

## 13. How to add a new skill

1. Decide scope: reusable → `global/skills/<id>/skill.md`; project-specific
   → `projects/<id>/skills/<id>/skill.md`.
2. Write frontmatter (`id`, `name`, `version`, `scope`, `description`,
   optional `tags`/`dependsOn`) matching that location.
3. Write the instructions as the Markdown body.
4. To override a global skill for one project, give it the same `id` under
   that project's `skills/` directory.

## 14. Installing the MCP server in a project

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

## 15. Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
cargo fmt --all

# semantic layer from the command line (same code path as the MCP tools)
cargo run -p nexus-server -- ingest acme-app
cargo run -p nexus-server -- export-graph acme-app
# open projects/acme-app/semantic/semantic-project.html in a browser
```

### Example project

`projects/acme-app` is a runnable example (used by the test suite) that
demonstrates every layer: a `checkout-ux` project skill depending on the
global `accessibility` skill, a `checkout-button` behavior spec, three graph
entities (`checkout-button`, `payment-flow`, `checkout-summary`) tied
together by `governed-by`/`uses`/`related-to`/`depends-on` relations,
`copilot`/`figma-make`/`human-reviewer` policies, the `checkout-submit`
semantic contract, and a `design/` folder (HTML design spec of the checkout
button, design tokens, a `Button` component spec). `projects/acme-app/semantic/`
holds the output of `ingest` + `export-graph`, including
`semantic-project.html`.

Tests live next to the code they cover (`#[cfg(test)] mod tests` in each
domain module) plus `crates/nexus-server/src/rpc.rs`, which walks the exact
scenario from the architecture doc — *"Create the checkout flow according
to our current UX rules"* — through the real JSON-RPC dispatcher, including
a call to the registered `graph_insights` tool.
`crates/nexus-semantic/tests/pipeline.rs` covers parsers, translation,
validation, diff, proposals and the end-to-end flow (repository with Git
history → register → inspect → validate → translate → diff → propose →
review → semantic graph → explorer export);
`crates/nexus-server/src/semantic_mcp_tests.rs` runs the same flow through
the JSON-RPC dispatcher, including `resources/list` and `resources/read`.
