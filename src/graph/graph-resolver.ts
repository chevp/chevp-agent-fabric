import type { GraphEntity, GraphRelation } from '../shared/types.js';
import type { GraphStore } from './graph-store.js';
import { getRelatedEntities, searchGraph } from './graph-query.js';

export interface ResolveGraphContextInput {
  projectId: string;
  /** Entity ids explicitly requested by the caller. */
  requestedEntities?: string[];
  /** Free-text task description used to seed a name/type search when no entities are requested. */
  task?: string;
  /** Hops to expand from each seed entity. Defaults to 1. */
  depth?: number;
}

/**
 * Resolves the minimal relevant slice of the Semantic Content Graph for a
 * task: the seed entities (requested, or found via search) plus their
 * immediate neighborhood.
 */
export async function resolveGraphContext(
  store: GraphStore,
  input: ResolveGraphContextInput,
): Promise<{ entities: GraphEntity[]; relations: GraphRelation[] }> {
  const seeds = new Set<string>(input.requestedEntities ?? []);

  if (seeds.size === 0 && input.task) {
    for (const word of tokenize(input.task)) {
      for (const entity of await searchGraph(store, input.projectId, word)) {
        seeds.add(entity.id);
      }
    }
  }

  const entities = new Map<string, GraphEntity>();
  const relations: GraphRelation[] = [];
  for (const seedId of seeds) {
    const seedEntity = await store.getEntity(input.projectId, seedId);
    if (seedEntity) entities.set(seedEntity.id, seedEntity);
    const related = await getRelatedEntities(store, input.projectId, seedId, {
      depth: input.depth ?? 1,
    });
    for (const entity of related.entities) entities.set(entity.id, entity);
    relations.push(...related.relations);
  }

  return { entities: [...entities.values()], relations };
}

function tokenize(text: string): string[] {
  return text
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((w) => w.length > 2);
}
