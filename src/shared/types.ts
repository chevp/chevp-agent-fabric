/**
 * Core domain types shared across Agent Nexus.
 *
 * These types describe the domain model independent of how data is
 * persisted (filesystem/Git today, a database potentially later) and
 * independent of the transport used to expose them (MCP, HTTP, ...).
 */

/** A client/agent talking to the Control Plane. Never hard-coded per-vendor. */
export interface ClientInfo {
  /** Stable identifier, e.g. "copilot", "figma-make", "ide-agent-x". */
  id: string;
  /** Coarse category, e.g. "coding-agent", "design-agent", "autonomous-agent". */
  type: string;
}

/** The bounded workspace and source of truth for a body of work. */
export interface Project {
  id: string;
  name: string;
  version: number;
  description?: string;
  /** Absolute path to the project's root directory on disk. */
  path: string;
}

export type SkillScope = 'global' | 'project';

/** Machine-readable frontmatter of a Markdown skill file. */
export interface SkillFrontmatter {
  id: string;
  name: string;
  version: string;
  scope: SkillScope;
  description: string;
  tags?: string[];
  /** IDs of other skills this skill depends on / builds upon. */
  dependsOn?: string[];
}

/** A fully loaded skill: frontmatter plus its instruction body. */
export interface Skill extends SkillFrontmatter {
  /** Markdown instructions for the consuming agent. */
  body: string;
  /** Present when the skill was loaded from a project directory. */
  projectId?: string;
  /** Absolute path the skill was loaded from, for diagnostics. */
  sourcePath: string;
}

export interface BehaviorTransition {
  from: string;
  event: string;
  to: string;
}

export interface BehaviorRule {
  id: string;
  description: string;
}

/** A structured, technology-agnostic UX/product behavior specification. */
export interface BehaviorSpec {
  id: string;
  type: string;
  version: number;
  states: string[];
  transitions: BehaviorTransition[];
  rules: BehaviorRule[];
  projectId: string;
  sourcePath: string;
}

/** A node in the Semantic Content Graph. */
export interface GraphEntity {
  id: string;
  type: string;
  name: string;
  /** Arbitrary additional descriptive fields declared on the entity. */
  attributes: Record<string, unknown>;
  projectId: string;
  sourcePath: string;
}

export const RELATION_KINDS = [
  'implements',
  'depends-on',
  'uses',
  'defined-by',
  'governed-by',
  'represented-by',
  'implemented-by',
  'validated-by',
  'related-to',
] as const;

export type RelationKind = (typeof RELATION_KINDS)[number];

/** A directed edge in the Semantic Content Graph. */
export interface GraphRelation {
  from: string;
  relation: RelationKind;
  to: string;
  projectId: string;
  sourcePath: string;
}

/** A single client's permission grant for a project (or globally). */
export interface PolicyRule {
  client: string;
  permissions: string[];
  projectId?: string;
  sourcePath: string;
}

/** An externally reachable system/tool exposed through the Control Plane. */
export interface Capability {
  id: string;
  name: string;
  description: string;
}

/** Input to the Context Resolver. */
export interface ContextRequest {
  projectId: string;
  task: string;
  client: ClientInfo;
  requestedEntities?: string[];
}

/** A single Markdown document under a project's `context/` directory. */
export interface ContextDocument {
  id: string;
  title: string;
  body: string;
  sourcePath: string;
}

/** The minimal relevant context package returned by the Context Resolver. */
export interface ResolvedContext {
  project: Project;
  contextDocuments: ContextDocument[];
  skills: Skill[];
  behaviors: BehaviorSpec[];
  entities: GraphEntity[];
  relations: GraphRelation[];
  policies: PolicyRule[];
}
