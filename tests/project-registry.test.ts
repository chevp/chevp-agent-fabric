import { describe, it, expect, afterEach } from 'vitest';
import path from 'node:path';
import { FsProjectStore } from '../src/projects/project-registry.js';
import { NotFoundError, ValidationError } from '../src/shared/errors.js';
import { createTmpRepo, writeFile } from './helpers/tmp-repo.js';

describe('FsProjectStore', () => {
  let cleanup: (() => Promise<void>) | undefined;
  afterEach(async () => {
    await cleanup?.();
    cleanup = undefined;
  });

  it('loads a project from project.yaml', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'project.yaml'),
      'id: acme-app\nname: Acme App\nversion: 1\n',
    );

    const store = new FsProjectStore(repo.projectsRoot);
    const project = await store.getProject('acme-app');

    expect(project).toEqual({
      id: 'acme-app',
      name: 'Acme App',
      version: 1,
      description: undefined,
      path: path.join(repo.projectsRoot, 'acme-app'),
    });
  });

  it('lists every project directory that has a project.yaml', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'a', 'project.yaml'),
      'id: a\nname: A\nversion: 1\n',
    );
    await writeFile(
      path.join(repo.projectsRoot, 'b', 'project.yaml'),
      'id: b\nname: B\nversion: 1\n',
    );
    // A stray directory without project.yaml must be ignored.
    await writeFile(path.join(repo.projectsRoot, 'not-a-project', 'notes.md'), 'hi');

    const store = new FsProjectStore(repo.projectsRoot);
    const projects = await store.listProjects();

    expect(projects.map((p) => p.id).sort()).toEqual(['a', 'b']);
  });

  it('returns undefined for a missing project', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    const store = new FsProjectStore(repo.projectsRoot);
    expect(await store.getProject('nope')).toBeUndefined();
  });

  it('requireProject throws NotFoundError for a missing project', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    const store = new FsProjectStore(repo.projectsRoot);
    await expect(store.requireProject('nope')).rejects.toBeInstanceOf(NotFoundError);
  });

  it('rejects invalid project data', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'broken', 'project.yaml'),
      'id: broken\nname: Broken\n', // missing required "version"
    );
    const store = new FsProjectStore(repo.projectsRoot);
    await expect(store.getProject('broken')).rejects.toBeInstanceOf(ValidationError);
  });
});
