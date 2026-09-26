import path from 'node:path';
import { ProjectFileSchema } from '../shared/schemas.js';
import { ValidationError } from '../shared/errors.js';
import { readYamlFile } from '../shared/fs-utils.js';
import type { Project } from '../shared/types.js';

/**
 * Loads and validates a single `project.yaml` file into a domain `Project`.
 * `projectDir` is the directory containing `project.yaml` (e.g. projects/acme-app).
 */
export async function loadProjectFile(projectDir: string): Promise<Project> {
  const filePath = path.join(projectDir, 'project.yaml');
  const raw = await readYamlFile(filePath);
  const result = ProjectFileSchema.safeParse(raw);
  if (!result.success) {
    throw new ValidationError(`Invalid project.yaml at ${filePath}`, result.error.issues);
  }
  const data = result.data;
  return {
    id: data.id,
    name: data.name,
    version: data.version,
    description: data.description,
    path: projectDir,
  };
}
