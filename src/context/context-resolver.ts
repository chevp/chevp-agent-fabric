import type { ContextRequest, ResolvedContext } from '../shared/types.js';
import type { FsProjectStore } from '../projects/project-registry.js';
import type { SkillRegistry } from '../skills/skill-registry.js';
import { resolveSkills } from '../skills/skill-resolver.js';
import type { BehaviorStore } from '../behavior/behavior-store.js';
import { resolveBehaviors } from '../behavior/behavior-resolver.js';
import type { GraphStore } from '../graph/graph-store.js';
import { resolveGraphContext } from '../graph/graph-resolver.js';
import type { PolicyEngine } from '../policies/policy-engine.js';
import { requirePermission } from '../policies/permission-checker.js';
import { loadContextDocuments } from './context-loader.js';

export interface ContextResolverDeps {
  projectsRoot: string;
  projectStore: FsProjectStore;
  skillRegistry: SkillRegistry;
  behaviorStore: BehaviorStore;
  graphStore: GraphStore;
  policyEngine: PolicyEngine;
}

/**
 * The Context Resolver is the heart of the Control Plane: given a task, a
 * project and a client, it determines the *minimal* relevant context —
 * never the entire project — by cross-referencing skills, behavior specs,
 * the semantic graph and policies.
 */
export class ContextResolver {
  constructor(private readonly deps: ContextResolverDeps) {}

  async resolveContext(request: ContextRequest): Promise<ResolvedContext> {
    const { projectsRoot, projectStore, skillRegistry, behaviorStore, graphStore, policyEngine } =
      this.deps;

    const project = await projectStore.requireProject(request.projectId);
    await requirePermission(policyEngine, request.client, 'read:project', project.id);

    const contextDocuments = await relevantContextDocuments(projectsRoot, project.id, request.task);

    const graphContext = await resolveGraphContext(graphStore, {
      projectId: project.id,
      requestedEntities: request.requestedEntities,
      task: request.task,
    });

    // Skills governing the resolved entities are pulled in explicitly, in
    // addition to whatever the task text itself matches.
    const governingSkillIds = graphContext.relations
      .filter((r) => r.relation === 'governed-by' || r.relation === 'implemented-by')
      .map((r) => r.to);

    const skills = await resolveSkills(skillRegistry, {
      projectId: project.id,
      task: request.task,
      explicitIds: governingSkillIds,
    });

    const behaviors = await resolveBehaviors(behaviorStore, {
      projectId: project.id,
      task: request.task,
      behaviorIds: graphContext.entities.map((e) => e.id),
    });

    const policies = await policyEngine.getPoliciesForClient(request.client.id, project.id);

    return {
      project,
      contextDocuments,
      skills,
      behaviors,
      entities: graphContext.entities,
      relations: graphContext.relations,
      policies,
    };
  }
}

async function relevantContextDocuments(projectsRoot: string, projectId: string, task: string) {
  const documents = await loadContextDocuments(projectsRoot, projectId);
  const taskWords = tokenize(task);
  return documents.filter((doc) => hasOverlap(tokenize(`${doc.title} ${doc.body}`), taskWords));
}

function hasOverlap(a: Set<string>, b: Set<string>): boolean {
  for (const word of a) if (b.has(word)) return true;
  return false;
}

function tokenize(text: string): Set<string> {
  return new Set(
    text
      .toLowerCase()
      .split(/[^a-z0-9]+/)
      .filter((w) => w.length > 2),
  );
}
