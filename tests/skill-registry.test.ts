import { describe, it, expect, afterEach } from 'vitest';
import path from 'node:path';
import { FsSkillStore, SkillRegistry } from '../src/skills/skill-registry.js';
import { resolveSkills } from '../src/skills/skill-resolver.js';
import { ValidationError, NotFoundError } from '../src/shared/errors.js';
import { createTmpRepo, writeFile } from './helpers/tmp-repo.js';

function skillFile(frontmatter: Record<string, unknown>, body = 'Body.'): string {
  const lines = Object.entries(frontmatter).map(([key, value]) => {
    if (Array.isArray(value)) {
      return `${key}:\n${value.map((v) => `  - ${v}`).join('\n')}`;
    }
    return `${key}: ${value}`;
  });
  return `---\n${lines.join('\n')}\n---\n\n${body}\n`;
}

describe('skills', () => {
  let cleanup: (() => Promise<void>) | undefined;
  afterEach(async () => {
    await cleanup?.();
    cleanup = undefined;
  });

  it('discovers global and project skills', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'accessibility', 'skill.md'),
      skillFile({
        id: 'accessibility',
        name: 'Accessibility',
        version: '1.0.0',
        scope: 'global',
        description: 'Baseline a11y rules',
      }),
    );
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'skills', 'checkout-ux', 'skill.md'),
      skillFile({
        id: 'checkout-ux',
        name: 'Checkout UX',
        version: '1.0.0',
        scope: 'project',
        description: 'Checkout rules',
      }),
    );

    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    const skills = await registry.listSkills('acme-app');

    expect(skills.map((s) => s.id).sort()).toEqual(['accessibility', 'checkout-ux']);
  });

  it('lets a project skill override a global skill of the same id', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'code-style', 'skill.md'),
      skillFile(
        {
          id: 'code-style',
          name: 'Code Style',
          version: '1.0.0',
          scope: 'global',
          description: 'Generic style',
        },
        'Generic rules.',
      ),
    );
    await writeFile(
      path.join(repo.projectsRoot, 'acme-app', 'skills', 'code-style', 'skill.md'),
      skillFile(
        {
          id: 'code-style',
          name: 'Code Style (Acme)',
          version: '2.0.0',
          scope: 'project',
          description: 'Acme-specific style',
        },
        'Acme rules.',
      ),
    );

    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    const skill = await registry.getSkill('code-style', 'acme-app');

    expect(skill.version).toBe('2.0.0');
    expect(skill.body).toBe('Acme rules.');
  });

  it('rejects a skill whose frontmatter scope does not match its directory', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    // Declared "project" scope but placed under global/skills.
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'mismatched', 'skill.md'),
      skillFile({
        id: 'mismatched',
        name: 'Mismatched',
        version: '1.0.0',
        scope: 'project',
        description: 'Should fail',
      }),
    );

    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    await expect(registry.listSkills()).rejects.toBeInstanceOf(ValidationError);
  });

  it('throws NotFoundError for an unknown skill id', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    await expect(registry.getSkill('nope')).rejects.toBeInstanceOf(NotFoundError);
  });

  it('resolves transitive dependsOn closures', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'a', 'skill.md'),
      skillFile({
        id: 'a',
        name: 'A',
        version: '1.0.0',
        scope: 'global',
        description: 'Depends on b',
        dependsOn: ['b'],
      }),
    );
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'b', 'skill.md'),
      skillFile({ id: 'b', name: 'B', version: '1.0.0', scope: 'global', description: 'Leaf' }),
    );

    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    const resolved = await resolveSkills(registry, { explicitIds: ['a'] });

    expect(resolved.map((s) => s.id).sort()).toEqual(['a', 'b']);
  });

  it('matches skills relevant to a task by keyword', async () => {
    const repo = await createTmpRepo();
    cleanup = repo.cleanup;
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'checkout-ux', 'skill.md'),
      skillFile({
        id: 'checkout-ux',
        name: 'Checkout UX',
        version: '1.0.0',
        scope: 'global',
        description: 'Rules for the checkout button',
        tags: ['checkout'],
      }),
    );
    await writeFile(
      path.join(repo.globalRoot, 'skills', 'unrelated', 'skill.md'),
      skillFile({
        id: 'unrelated',
        name: 'Unrelated',
        version: '1.0.0',
        scope: 'global',
        description: 'Something else entirely',
      }),
    );

    const registry = new SkillRegistry(new FsSkillStore(repo.globalRoot, repo.projectsRoot));
    const resolved = await resolveSkills(registry, { task: 'Implement the checkout button' });

    expect(resolved.map((s) => s.id)).toEqual(['checkout-ux']);
  });
});
