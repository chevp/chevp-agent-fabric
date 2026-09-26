import { promises as fs } from 'node:fs';
import path from 'node:path';
import yaml from 'js-yaml';
import { ValidationError } from './errors.js';

/** Returns true if a path exists (file or directory). */
export async function pathExists(p: string): Promise<boolean> {
  try {
    await fs.access(p);
    return true;
  } catch {
    return false;
  }
}

/** Lists immediate subdirectories of `dir`. Returns [] if `dir` doesn't exist. */
export async function listSubdirectories(dir: string): Promise<string[]> {
  if (!(await pathExists(dir))) return [];
  const entries = await fs.readdir(dir, { withFileTypes: true });
  return entries.filter((e) => e.isDirectory()).map((e) => e.name);
}

/** Recursively lists files matching `extensions` under `dir`. Returns [] if `dir` doesn't exist. */
export async function listFilesRecursive(dir: string, extensions: string[]): Promise<string[]> {
  if (!(await pathExists(dir))) return [];
  const results: string[] = [];
  const walk = async (current: string): Promise<void> => {
    const entries = await fs.readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        await walk(full);
      } else if (extensions.includes(path.extname(entry.name))) {
        results.push(full);
      }
    }
  };
  await walk(dir);
  return results;
}

/** Reads and parses a YAML file, throwing ValidationError on malformed YAML. */
export async function readYamlFile(filePath: string): Promise<unknown> {
  const raw = await fs.readFile(filePath, 'utf-8');
  try {
    return yaml.load(raw);
  } catch (err) {
    throw new ValidationError(`Malformed YAML in ${filePath}: ${(err as Error).message}`, err);
  }
}
