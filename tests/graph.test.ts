import { describe, it, expect, afterEach } from 'vitest';
import path from 'node:path';
import { FsGraphStore } from '../src/graph/graph-store.js';
import { findPath, getRelatedEntities, searchGraph } from '../src/graph/graph-query.js';
import { ValidationError, VersionConflictError } from '../src/shared/errors.js';
import { createTmpRepo, writeFile } from './helpers/tmp-repo.js';

async function seedChainGraph(projectsRoot: string): Promise<void> {
  const dir = path.join(projectsRoot, 'acme-app', 'graph');
  await writeFile(path.join(dir, 'entities', 'a.yaml'), 'id: a\ntype: component\nname: A\n');
  await writeFile(path.join(dir, 'entities', 'b.yaml'), 'id: b\ntype: component\nname: B\n');
  await writeFile(path.join(dir, 'entities', 'c.yaml'), 'id: c\ntype: component\nname: C\n');
  await writeFile(path.join(dir, 'relations', 'a-uses-b.yaml'), 'from: a\nrelation: uses\nto: b\n');
  await writeFile(path.join(dir, 'relations', 'b-uses-c.yaml'), 'from: b\nrelation: uses\nto: c\n');
}

describe('Semantic Content Graph', () => {
  let cleanup: (() => Promise<void>) | undefined;
  afterEach(async () => {
    await cleanup?.();
    cleanup = undefined;
  });

  it('traverses related entities up to a given depth', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await seedChainGraph(repo.projectsRoot);
    const store = new FsGraphStore(repo.projectsRoot);

    const depth1 = await getRelatedEntities(store, 'acme-app', 'a', { depth: 1 });
    expect(depth1.entities.map((e) => e.id)).toEqual(['b']);

    const depth2 = await getRelatedEntities(store, 'acme-app', 'a', { depth: 2 });
    expect(depth2.entities.map((e) => e.id).sort()).toEqual(['b', 'c']);
  });

  it('finds a path between two entities', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await seedChainGraph(repo.projectsRoot);
    const store = new FsGraphStore(repo.projectsRoot);

    const path_ = await findPath(store, 'acme-app', 'a', 'c');
    expect(path_?.map((r) => `${r.from}->${r.to}`)).toEqual(['a->b', 'b->c']);
  });

  it('returns undefined when no path exists', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'graph', 'entities', 'isolated.yaml'),
      'id: isolated\ntype: component\nname: Isolated\n',
    );
    await seedChainGraph(repo.projectsRoot);
    const store = new FsGraphStore(repo.projectsRoot);

    expect(await findPath(store, 'acme-app', 'a', 'isolated')).toBeUndefined();
  });

  it('searches entities by id, name and type', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await seedChainGraph(repo.projectsRoot);
    const store = new FsGraphStore(repo.projectsRoot);

    const results = await searchGraph(store, 'acme-app', 'component');
    expect(results.map((e) => e.id).sort()).toEqual(['a', 'b', 'c']);
  });

  it('rejects a relation whose endpoints are both unknown entities', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'graph', 'entities', 'a.yaml'),
      'id: a\ntype: component\nname: A\n',
    );
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'graph', 'relations', 'bad.yaml'),
      'from: ghost-one\nrelation: uses\nto: ghost-two\n',
    );
    const store = new FsGraphStore(repo.projectsRoot);

    await expect(store.listRelations('acme-app')).rejects.toBeInstanceOf(ValidationError);
  });

  it('rejects duplicate entity ids', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'graph', 'entities', 'one.yaml'),
      'id: dup\ntype: component\nname: One\n',
    );
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'graph', 'entities', 'two.yaml'),
      'id: dup\ntype: component\nname: Two\n',
    );
    const store = new FsGraphStore(repo.projectsRoot);

    await expect(store.listEntities('acme-app')).rejects.toBeInstanceOf(VersionConflictError);
  });
});
