import path from 'node:path';
import { listSubdirectories, pathExists } from '../shared/fs-utils.js';
import { NotFoundError } from '../shared/errors.js';
import type { Project } from '../shared/types.js';
import { loadProjectFile } from './project-loader.js';

/**
 * Storage abstraction for projects. The Git/filesystem implementation is the
 * only one that exists in V1; a database-backed implementation can be added
 * later without touching any application service that depends on this
 * interface.
 */
export interface ProjectStore {
  listProjects(): Promise<Project[]>;
  getProject(id: string): Promise<Project | undefined>;
}

/** Filesystem/Git-backed implementation: one directory per project under `projectsRoot`. */
export class FsProjectStore implements ProjectStore {
  constructor(private readonly projectsRoot: string) {}

  async listProjects(): Promise<Project[]> {
    const dirs = await listSubdirectories(this.projectsRoot);
    const projects: Project[] = [];
    for (const dir of dirs) {
      const projectDir = path.join(this.projectsRoot, dir);
      if (await pathExists(path.join(projectDir, 'project.yaml'))) {
        projects.push(await loadProjectFile(projectDir));
      }
    }
    return projects;
  }

  async getProject(id: string): Promise<Project | undefined> {
    const projectDir = path.join(this.projectsRoot, id);
    if (!(await pathExists(path.join(projectDir, 'project.yaml')))) {
      return undefined;
    }
    return loadProjectFile(projectDir);
  }

  async requireProject(id: string): Promise<Project> {
    const project = await this.getProject(id);
    if (!project) throw new NotFoundError('Project', id);
    return project;
  }
}
