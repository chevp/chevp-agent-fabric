import path from 'node:path';
import { promises as fs } from 'node:fs';
import { listFilesRecursive } from '../shared/fs-utils.js';
import type { ContextDocument } from '../shared/types.js';

/** Loads all project knowledge/documentation Markdown files under `context/`. */
export async function loadContextDocuments(
  projectsRoot: string,
  projectId: string,
): Promise<ContextDocument[]> {
  const dir = path.join(projectsRoot, projectId, 'context');
  const files = await listFilesRecursive(dir, ['.md']);
  return Promise.all(
    files.map(async (filePath) => {
      const raw = await fs.readFile(filePath, 'utf-8');
      const titleMatch = raw.match(/^#\s+(.+)$/m);
      return {
        id: path.relative(dir, filePath),
        title: titleMatch?.[1]?.trim() ?? path.basename(filePath, '.md'),
        body: raw.trim(),
        sourcePath: filePath,
      };
    }),
  );
}
