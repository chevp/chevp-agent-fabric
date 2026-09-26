import { promises as fs } from 'node:fs';
import matter from 'gray-matter';
import { SkillFrontmatterSchema } from '../shared/schemas.js';
import { ValidationError } from '../shared/errors.js';
import type { Skill, SkillScope } from '../shared/types.js';

/**
 * Loads a single Markdown skill file (frontmatter + body) into a domain `Skill`.
 * `expectedScope` is verified against the frontmatter to catch misplaced files
 * (e.g. a project-scoped skill accidentally declared under global/skills).
 */
export async function loadSkillFile(
  filePath: string,
  expectedScope: SkillScope,
  projectId?: string,
): Promise<Skill> {
  const raw = await fs.readFile(filePath, 'utf-8');
  const parsed = matter(raw);
  const result = SkillFrontmatterSchema.safeParse(parsed.data);
  if (!result.success) {
    throw new ValidationError(`Invalid skill frontmatter in ${filePath}`, result.error.issues);
  }
  const frontmatter = result.data;
  if (frontmatter.scope !== expectedScope) {
    throw new ValidationError(
      `Skill "${frontmatter.id}" at ${filePath} declares scope "${frontmatter.scope}" but is located under a "${expectedScope}" directory`,
      { filePath, declaredScope: frontmatter.scope, expectedScope },
    );
  }
  return {
    ...frontmatter,
    body: parsed.content.trim(),
    projectId: expectedScope === 'project' ? projectId : undefined,
    sourcePath: filePath,
  };
}
