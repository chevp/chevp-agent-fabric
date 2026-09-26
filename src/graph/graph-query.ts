import type { GraphEntity, GraphRelation } from '../shared/types.js';
import type { GraphStore } from './graph-store.js';

export interface RelatedEntitiesOptions {
  /** How many relation hops to traverse. Defaults to 1. */
  depth?: number;
  /** Restrict traversal to these relation kinds; defaults to all kinds. */
  relationKinds?: string[];
}

export async function getEntity(
  store: GraphStore,
  projectId: string,
  id: string,
): Promise<GraphEntity | undefined> {
  return store.getEntity(projectId, id);
}

/** Breadth-first traversal from `entityId` up to `depth` hops, in either direction. */
export async function getRelatedEntities(
  store: GraphStore,
  projectId: string,
  entityId: string,
  options: RelatedEntitiesOptions = {},
): Promise<{ entities: GraphEntity[]; relations: GraphRelation[] }> {
  const depth = options.depth ?? 1;
  const allEntities = await store.listEntities(projectId);
  const allRelations = await store.listRelations(projectId);
  const entityById = new Map(allEntities.map((e) => [e.id, e]));
  const relationsToUse = options.relationKinds
    ? allRelations.filter((r) => options.relationKinds!.includes(r.relation))
    : allRelations;

  const visited = new Set<string>([entityId]);
  const foundEntities = new Map<string, GraphEntity>();
  const foundRelations: GraphRelation[] = [];
  let frontier = [entityId];

  for (let hop = 0; hop < depth && frontier.length > 0; hop++) {
    const next: string[] = [];
    for (const current of frontier) {
      for (const relation of relationsToUse) {
        const neighbor =
          relation.from === current
            ? relation.to
            : relation.to === current
              ? relation.from
              : undefined;
        if (neighbor === undefined) continue;
        foundRelations.push(relation);
        const neighborEntity = entityById.get(neighbor);
        if (neighborEntity) foundEntities.set(neighborEntity.id, neighborEntity);
        if (!visited.has(neighbor)) {
          visited.add(neighbor);
          next.push(neighbor);
        }
      }
    }
    frontier = next;
  }

  foundEntities.delete(entityId);
  return { entities: [...foundEntities.values()], relations: dedupeRelations(foundRelations) };
}

/** Shortest path (in hops) between two entities, following relations in either direction. */
export async function findPath(
  store: GraphStore,
  projectId: string,
  fromId: string,
  toId: string,
): Promise<GraphRelation[] | undefined> {
  if (fromId === toId) return [];
  const relations = await store.listRelations(projectId);
  const adjacency = new Map<string, GraphRelation[]>();
  for (const relation of relations) {
    addAdjacency(adjacency, relation.from, relation);
    addAdjacency(adjacency, relation.to, relation);
  }

  const visited = new Set<string>([fromId]);
  const queue: { node: string; path: GraphRelation[] }[] = [{ node: fromId, path: [] }];

  while (queue.length > 0) {
    const { node, path } = queue.shift()!;
    for (const relation of adjacency.get(node) ?? []) {
      const neighbor = relation.from === node ? relation.to : relation.from;
      if (visited.has(neighbor)) continue;
      const newPath = [...path, relation];
      if (neighbor === toId) return newPath;
      visited.add(neighbor);
      queue.push({ node: neighbor, path: newPath });
    }
  }
  return undefined;
}

/** Case-insensitive search over entity id, name and type. */
export async function searchGraph(
  store: GraphStore,
  projectId: string,
  query: string,
): Promise<GraphEntity[]> {
  const needle = query.toLowerCase();
  const entities = await store.listEntities(projectId);
  return entities.filter(
    (e) =>
      e.id.toLowerCase().includes(needle) ||
      e.name.toLowerCase().includes(needle) ||
      e.type.toLowerCase().includes(needle),
  );
}

function addAdjacency(
  map: Map<string, GraphRelation[]>,
  key: string,
  relation: GraphRelation,
): void {
  const list = map.get(key) ?? [];
  list.push(relation);
  map.set(key, list);
}

function dedupeRelations(relations: GraphRelation[]): GraphRelation[] {
  const seen = new Set<string>();
  const result: GraphRelation[] = [];
  for (const relation of relations) {
    const key = `${relation.from}|${relation.relation}|${relation.to}`;
    if (!seen.has(key)) {
      seen.add(key);
      result.push(relation);
    }
  }
  return result;
}
