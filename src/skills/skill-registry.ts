import path from 'node:path';
import { listFilesRecursive } from '../shared/fs-utils.js';
import { NotFoundError, VersionConflictError } from '../shared/errors.js';
import type { Skill } from '../shared/types.js';
import { loadSkillFile } from './skill-loader.js';

/** Storage abstraction for skills. Filesystem/Git-backed in V1. */
export interface SkillStore {
  /** All global skills. */
  listGlobalSkills(): Promise<Skill[]>;
  /** All skills declared under a specific project. */
  listProjectSkills(projectId: string): Promise<Skill[]>;
}

export class FsSkillStore implements SkillStore {
  constructor(
    private readonly globalRoot: string,
    private readonly projectsRoot: string,
  ) {}

  async listGlobalSkills(): Promise<Skill[]> {
    const files = await listFilesRecursive(path.join(this.globalRoot, 'skills'), ['.md']);
    const skills = await Promise.all(files.map((f) => loadSkillFile(f, 'global')));
    assertNoDuplicateIds(skills);
    return skills;
  }

  async listProjectSkills(projectId: string): Promise<Skill[]> {
    const projectSkillsDir = path.join(this.projectsRoot, projectId, 'skills');
    const files = await listFilesRecursive(projectSkillsDir, ['.md']);
    const skills = await Promise.all(files.map((f) => loadSkillFile(f, 'project', projectId)));
    assertNoDuplicateIds(skills);
    return skills;
  }
}

function assertNoDuplicateIds(skills: Skill[]): void {
  const seen = new Map<string, Skill>();
  for (const skill of skills) {
    const existing = seen.get(skill.id);
    if (existing) {
      throw new VersionConflictError(
        `Duplicate skill id "${skill.id}" found at both ${existing.sourcePath} and ${skill.sourcePath}`,
      );
    }
    seen.set(skill.id, skill);
  }
}

/** Application-facing read API combining the store with simple lookup helpers. */
export class SkillRegistry {
  constructor(private readonly store: SkillStore) {}

  async listSkills(projectId?: string): Promise<Skill[]> {
    const global = await this.store.listGlobalSkills();
    if (!projectId) return global;
    const project = await this.store.listProjectSkills(projectId);
    return mergeWithProjectOverrides(global, project);
  }

  async getSkill(id: string, projectId?: string): Promise<Skill> {
    const skills = await this.listSkills(projectId);
    const skill = skills.find((s) => s.id === id);
    if (!skill) throw new NotFoundError('Skill', id);
    return skill;
  }
}

/** Project skills override global skills declared under the same id. */
export function mergeWithProjectOverrides(global: Skill[], project: Skill[]): Skill[] {
  const merged = new Map<string, Skill>();
  for (const skill of global) merged.set(skill.id, skill);
  for (const skill of project) merged.set(skill.id, skill);
  return [...merged.values()];
}
