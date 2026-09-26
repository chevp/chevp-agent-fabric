import path from 'node:path';
import { listFilesRecursive, readYamlFile } from '../shared/fs-utils.js';
import { BehaviorFileSchema } from '../shared/schemas.js';
import { NotFoundError, ValidationError } from '../shared/errors.js';
import type { BehaviorSpec } from '../shared/types.js';

/** Storage abstraction for behavior specifications. Filesystem/Git-backed in V1. */
export interface BehaviorStore {
  listBehaviors(projectId: string): Promise<BehaviorSpec[]>;
  getBehavior(projectId: string, id: string): Promise<BehaviorSpec | undefined>;
}

export class FsBehaviorStore implements BehaviorStore {
  constructor(private readonly projectsRoot: string) {}

  async listBehaviors(projectId: string): Promise<BehaviorSpec[]> {
    const dir = path.join(this.projectsRoot, projectId, 'behavior');
    const files = await listFilesRecursive(dir, ['.yaml', '.yml']);
    return Promise.all(files.map((f) => loadBehaviorFile(f, projectId)));
  }

  async getBehavior(projectId: string, id: string): Promise<BehaviorSpec | undefined> {
    const behaviors = await this.listBehaviors(projectId);
    return behaviors.find((b) => b.id === id);
  }

  async requireBehavior(projectId: string, id: string): Promise<BehaviorSpec> {
    const behavior = await this.getBehavior(projectId, id);
    if (!behavior) throw new NotFoundError('Behavior', id);
    return behavior;
  }
}

async function loadBehaviorFile(filePath: string, projectId: string): Promise<BehaviorSpec> {
  const raw = await readYamlFile(filePath);
  const result = BehaviorFileSchema.safeParse(raw);
  if (!result.success) {
    throw new ValidationError(`Invalid behavior spec at ${filePath}`, result.error.issues);
  }
  const data = result.data;
  validateTransitions(data, filePath);
  return { ...data, projectId, sourcePath: filePath };
}

/** Every transition must reference states declared in `states`. */
function validateTransitions(
  data: { states: string[]; transitions: { from: string; to: string }[] },
  filePath: string,
): void {
  const stateSet = new Set(data.states);
  for (const transition of data.transitions) {
    if (!stateSet.has(transition.from) || !stateSet.has(transition.to)) {
      throw new ValidationError(
        `Behavior spec at ${filePath} has a transition referencing an undeclared state`,
        transition,
      );
    }
  }
}
