import { describe, it, expect, afterEach } from 'vitest';
import path from 'node:path';
import { FsBehaviorStore } from '../src/behavior/behavior-store.js';
import { ValidationError, NotFoundError } from '../src/shared/errors.js';
import { createTmpRepo, writeFile } from './helpers/tmp-repo.js';

const VALID_BEHAVIOR = `
id: checkout-button
type: ui-behavior
version: 1
states:
  - idle
  - loading
  - success
  - error
transitions:
  - from: idle
    event: submit
    to: loading
  - from: loading
    event: success
    to: success
  - from: loading
    event: failure
    to: error
rules:
  - id: prevent-double-submit
    description: User cannot submit twice while loading
`;

describe('FsBehaviorStore', () => {
  let cleanup: (() => Promise<void>) | undefined;
  afterEach(async () => {
    await cleanup?.();
    cleanup = undefined;
  });

  it('parses a valid behavior spec', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'behavior', 'checkout-button.yaml'),
      VALID_BEHAVIOR,
    );

    const store = new FsBehaviorStore(repo.projectsRoot);
    const behavior = await store.getBehavior('acme-app', 'checkout-button');

    expect(behavior?.states).toEqual(['idle', 'loading', 'success', 'error']);
    expect(behavior?.transitions).toHaveLength(3);
    expect(behavior?.rules[0]?.id).toBe('prevent-double-submit');
  });

  it('rejects a transition referencing an undeclared state', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'behavior', 'broken.yaml'),
      `
id: broken
type: ui-behavior
version: 1
states:
  - idle
transitions:
  - from: idle
    event: submit
    to: nonexistent-state
`,
    );

    const store = new FsBehaviorStore(repo.projectsRoot);
    await expect(store.listBehaviors('acme-app')).rejects.toBeInstanceOf(ValidationError);
  });

  it('throws NotFoundError for a missing behavior', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    const store = new FsBehaviorStore(repo.projectsRoot);
    await expect(store.requireBehavior('acme-app', 'nope')).rejects.toBeInstanceOf(NotFoundError);
  });
});
