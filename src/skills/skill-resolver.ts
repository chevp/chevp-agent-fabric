import { NotFoundError } from '../shared/errors.js';
import type { Skill } from '../shared/types.js';
import type { SkillRegistry } from './skill-registry.js';

export interface ResolveSkillsInput {
  projectId?: string;
  /** Free-text task description used for keyword-based relevance matching. */
  task?: string;
  /** Skill ids the caller already knows are relevant (e.g. from the graph). */
  explicitIds?: string[];
}

/**
 * Resolves the set of skills relevant to a task.
 *
 * Order of precedence/inclusion:
 * 1. global skills matched by relevance
 * 2. project skills matched by relevance (override same-id global skills)
 * 3. explicitly requested skill ids
 * 4. transitive `dependsOn` closure
 * 5. project-scoped versions win over global for the same id (versioning)
 */
export async function resolveSkills(
  registry: SkillRegistry,
  input: ResolveSkillsInput,
): Promise<Skill[]> {
  const all = await registry.listSkills(input.projectId);
  const byId = new Map(all.map((s) => [s.id, s]));

  const relevant = new Set<string>();
  for (const id of input.explicitIds ?? []) relevant.add(id);

  if (input.task) {
    const taskWords = tokenize(input.task);
    for (const skill of all) {
      if (isRelevant(skill, taskWords)) relevant.add(skill.id);
    }
  }

  // Transitive dependency closure.
  const queue = [...relevant];
  while (queue.length > 0) {
    const id = queue.pop();
    if (id === undefined) continue;
    const skill = byId.get(id);
    if (!skill) throw new NotFoundError('Skill', id);
    for (const dep of skill.dependsOn ?? []) {
      if (!relevant.has(dep)) {
        relevant.add(dep);
        queue.push(dep);
      }
    }
  }

  return [...relevant].map((id) => {
    const skill = byId.get(id);
    if (!skill) throw new NotFoundError('Skill', id);
    return skill;
  });
}

function isRelevant(skill: Skill, taskWords: Set<string>): boolean {
  const haystack = tokenize(
    [skill.id, skill.name, skill.description, ...(skill.tags ?? [])].join(' '),
  );
  for (const word of taskWords) {
    if (haystack.has(word)) return true;
  }
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
