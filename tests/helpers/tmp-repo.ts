import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

/** Creates an isolated temp directory with `projects/` and `global/` roots for tests. */
export async function createTmpRepo(): Promise<{
  root: string;
  projectsRoot: string;
  globalRoot: string;
  cleanup: () => Promise<void>;
}> {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'agent-nexus-test-'));
  const projectsRoot = path.join(root, 'projects');
  const globalRoot = path.join(root, 'global');
  await fs.mkdir(projectsRoot, { recursive: true });
  await fs.mkdir(globalRoot, { recursive: true });
  return {
    root,
    projectsRoot,
    globalRoot,
    cleanup: () => fs.rm(root, { recursive: true, force: true }),
  };
}

export async function writeFile(filePath: string, content: string): Promise<void> {
  await fs.mkdir(path.dirname(filePath), { recursive: true });
  await fs.writeFile(filePath, content, 'utf-8');
}
