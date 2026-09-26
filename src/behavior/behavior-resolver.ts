import type { BehaviorSpec } from '../shared/types.js';
import type { BehaviorStore } from './behavior-store.js';

export interface ResolveBehaviorsInput {
  projectId: string;
  /** Behavior ids known to be relevant (e.g. resolved from the graph). */
  behaviorIds?: string[];
  /** Free-text task description used for keyword-based relevance matching. */
  task?: string;
}

/** Resolves the behavior specs relevant to a task, without loading the whole project. */
export async function resolveBehaviors(
  store: BehaviorStore,
  input: ResolveBehaviorsInput,
): Promise<BehaviorSpec[]> {
  const all = await store.listBehaviors(input.projectId);
  const relevant = new Set<string>(input.behaviorIds ?? []);

  if (input.task) {
    const taskWords = tokenize(input.task);
    for (const behavior of all) {
      if (tokenize(behavior.id).size > 0 && hasOverlap(tokenize(behavior.id), taskWords)) {
        relevant.add(behavior.id);
      }
    }
  }

  return all.filter((b) => relevant.has(b.id));
}

function hasOverlap(a: Set<string>, b: Set<string>): boolean {
  for (const word of a) if (b.has(word)) return true;
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
