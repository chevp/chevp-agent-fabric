import path from 'node:path';
import { FsProjectStore } from '../projects/project-registry.js';
import { FsSkillStore, SkillRegistry } from '../skills/skill-registry.js';
import { resolveSkills } from '../skills/skill-resolver.js';
import { FsBehaviorStore } from '../behavior/behavior-store.js';
import { FsGraphStore } from '../graph/graph-store.js';
import { getEntity, getRelatedEntities, searchGraph } from '../graph/graph-query.js';
import { FsPolicyStore, PolicyEngine } from '../policies/policy-engine.js';
import { requirePermission } from '../policies/permission-checker.js';
import { CapabilityRegistry } from '../capabilities/capability-registry.js';
import { ContextResolver } from '../context/context-resolver.js';
import type {
  BehaviorSpec,
  ClientInfo,
  ContextRequest,
  GraphEntity,
  PolicyRule,
  Project,
  ResolvedContext,
  Skill,
} from '../shared/types.js';

export interface NexusServicesOptions {
  /** Root directory containing project directories (default: "<repoRoot>/projects"). */
  projectsRoot: string;
  /** Root directory containing global skills/policies (default: "<repoRoot>/global"). */
  globalRoot: string;
}

/**
 * The single application-service layer. Every transport (MCP, HTTP, tests)
 * calls into this class; it is the only thing that talks to the stores and
 * resolvers. No business logic lives in `src/server`.
 */
export class NexusServices {
  readonly projectStore: FsProjectStore;
  readonly skillRegistry: SkillRegistry;
  readonly behaviorStore: FsBehaviorStore;
  readonly graphStore: FsGraphStore;
  readonly policyEngine: PolicyEngine;
  readonly capabilityRegistry: CapabilityRegistry;
  readonly contextResolver: ContextResolver;

  constructor(private readonly options: NexusServicesOptions) {
    this.projectStore = new FsProjectStore(options.projectsRoot);
    this.skillRegistry = new SkillRegistry(
      new FsSkillStore(options.globalRoot, options.projectsRoot),
    );
    this.behaviorStore = new FsBehaviorStore(options.projectsRoot);
    this.graphStore = new FsGraphStore(options.projectsRoot);
    this.policyEngine = new PolicyEngine(
      new FsPolicyStore(options.globalRoot, options.projectsRoot),
    );
    this.capabilityRegistry = new CapabilityRegistry();
    this.contextResolver = new ContextResolver({
      projectsRoot: options.projectsRoot,
      projectStore: this.projectStore,
      skillRegistry: this.skillRegistry,
      behaviorStore: this.behaviorStore,
      graphStore: this.graphStore,
      policyEngine: this.policyEngine,
    });
  }

  static fromRepoRoot(repoRoot: string): NexusServices {
    return new NexusServices({
      projectsRoot: path.join(repoRoot, 'projects'),
      globalRoot: path.join(repoRoot, 'global'),
    });
  }

  // --- Projects ------------------------------------------------------------

  async listProjects(): Promise<Project[]> {
    return this.projectStore.listProjects();
  }

  async getProject(id: string): Promise<Project> {
    return this.projectStore.requireProject(id);
  }

  // --- Skills ----------------------------------------------------------------

  async listSkills(projectId?: string): Promise<Skill[]> {
    return this.skillRegistry.listSkills(projectId);
  }

  async getSkill(id: string, projectId?: string): Promise<Skill> {
    return this.skillRegistry.getSkill(id, projectId);
  }

  async resolveSkills(input: { projectId?: string; task?: string; explicitIds?: string[] }) {
    return resolveSkills(this.skillRegistry, input);
  }

  // --- Behavior ----------------------------------------------------------------

  async listBehaviors(projectId: string): Promise<BehaviorSpec[]> {
    return this.behaviorStore.listBehaviors(projectId);
  }

  async getBehavior(projectId: string, id: string): Promise<BehaviorSpec> {
    return this.behaviorStore.requireBehavior(projectId, id);
  }

  // --- Graph ----------------------------------------------------------------

  async getEntity(projectId: string, id: string): Promise<GraphEntity | undefined> {
    return getEntity(this.graphStore, projectId, id);
  }

  async searchGraph(projectId: string, query: string): Promise<GraphEntity[]> {
    return searchGraph(this.graphStore, projectId, query);
  }

  async getRelatedEntities(projectId: string, id: string, depth?: number) {
    return getRelatedEntities(this.graphStore, projectId, id, { depth });
  }

  // --- Policies ----------------------------------------------------------------

  async getProjectPolicy(clientId: string, projectId: string): Promise<PolicyRule[]> {
    return this.policyEngine.getPoliciesForClient(clientId, projectId);
  }

  async checkPermission(
    client: ClientInfo,
    permission: string,
    projectId?: string,
  ): Promise<boolean> {
    return this.policyEngine.hasPermission(client.id, permission, projectId);
  }

  async requirePermission(
    client: ClientInfo,
    permission: string,
    projectId?: string,
  ): Promise<void> {
    await requirePermission(this.policyEngine, client, permission, projectId);
  }

  // --- Context ----------------------------------------------------------------

  async resolveContext(request: ContextRequest): Promise<ResolvedContext> {
    return this.contextResolver.resolveContext(request);
  }
}
