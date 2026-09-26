import path from 'node:path';
import { listFilesRecursive, readYamlFile } from '../shared/fs-utils.js';
import { EntityFileSchema, RelationFileSchema } from '../shared/schemas.js';
import { ValidationError, VersionConflictError } from '../shared/errors.js';
import type { GraphEntity, GraphRelation } from '../shared/types.js';

/**
 * Storage abstraction for the Semantic Content Graph. Filesystem/Git-backed
 * in V1; a graph database (e.g. Neo4j) can implement the same interface
 * later without changing GraphQuery/GraphResolver or any MCP handler.
 */
export interface GraphStore {
  listEntities(projectId: string): Promise<GraphEntity[]>;
  listRelations(projectId: string): Promise<GraphRelation[]>;
  getEntity(projectId: string, id: string): Promise<GraphEntity | undefined>;
}

export class FsGraphStore implements GraphStore {
  constructor(private readonly projectsRoot: string) {}

  async listEntities(projectId: string): Promise<GraphEntity[]> {
    const dir = path.join(this.projectsRoot, projectId, 'graph', 'entities');
    const files = await listFilesRecursive(dir, ['.yaml', '.yml']);
    const entities = await Promise.all(files.map((f) => loadEntityFile(f, projectId)));
    assertNoDuplicateEntityIds(entities);
    return entities;
  }

  async listRelations(projectId: string): Promise<GraphRelation[]> {
    const dir = path.join(this.projectsRoot, projectId, 'graph', 'relations');
    const files = await listFilesRecursive(dir, ['.yaml', '.yml']);
    const relations = await Promise.all(files.map((f) => loadRelationFile(f, projectId)));
    const entityIds = new Set((await this.listEntities(projectId)).map((e) => e.id));
    for (const relation of relations) {
      assertEndpointIsResolvable(relation, entityIds);
    }
    return relations;
  }

  async getEntity(projectId: string, id: string): Promise<GraphEntity | undefined> {
    const entities = await this.listEntities(projectId);
    return entities.find((e) => e.id === id);
  }
}

async function loadEntityFile(filePath: string, projectId: string): Promise<GraphEntity> {
  const raw = await readYamlFile(filePath);
  const result = EntityFileSchema.safeParse(raw);
  if (!result.success) {
    throw new ValidationError(`Invalid graph entity at ${filePath}`, result.error.issues);
  }
  const { id, type, name, ...attributes } = result.data;
  return { id, type, name, attributes, projectId, sourcePath: filePath };
}

async function loadRelationFile(filePath: string, projectId: string): Promise<GraphRelation> {
  const raw = await readYamlFile(filePath);
  const result = RelationFileSchema.safeParse(raw);
  if (!result.success) {
    throw new ValidationError(`Invalid graph relation at ${filePath}`, result.error.issues);
  }
  return { ...result.data, projectId, sourcePath: filePath };
}

function assertNoDuplicateEntityIds(entities: GraphEntity[]): void {
  const seen = new Map<string, GraphEntity>();
  for (const entity of entities) {
    const existing = seen.get(entity.id);
    if (existing) {
      throw new VersionConflictError(
        `Duplicate entity id "${entity.id}" found at both ${existing.sourcePath} and ${entity.sourcePath}`,
      );
    }
    seen.set(entity.id, entity);
  }
}

/**
 * A relation may point at an entity, a skill, or another graph object outside
 * this store's view; we only fail closed when neither endpoint resolves to a
 * known entity, since relations can legitimately target skills/policies too.
 */
function assertEndpointIsResolvable(relation: GraphRelation, entityIds: Set<string>): void {
  if (!entityIds.has(relation.from) && !entityIds.has(relation.to)) {
    throw new ValidationError(
      `Relation at ${relation.sourcePath} references neither a known entity as "from" nor "to" (${relation.from} -> ${relation.to})`,
      relation,
    );
  }
}
