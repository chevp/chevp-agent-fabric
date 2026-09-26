/**
 * Level-4 workflow interfaces.
 *
 * These types exist so the domain model has a stable shape to grow into,
 * e.g.:
 *
 *   UX Definition -> Behavior Specification -> Figma -> Implementation
 *     -> Tests -> Verification -> Human Approval
 *
 * V1 does NOT implement a workflow engine, scheduler, or executor. Nothing
 * in `src/server`, `src/context`, or any store depends on these types today.
 * They are a deliberate placeholder for a future phase.
 */

export type WorkflowStateName = string;

export interface WorkflowState {
  name: WorkflowStateName;
  description?: string;
}

/** A declarative process definition: an ordered/graph set of states a task moves through. */
export interface Workflow {
  id: string;
  name: string;
  version: number;
  states: WorkflowState[];
  initialState: WorkflowStateName;
}

/** A unit of work to be carried out by one or more agents, tracked against a Workflow. */
export interface Task {
  id: string;
  workflowId: string;
  projectId: string;
  description: string;
  currentState: WorkflowStateName;
}

/** A single attempt at advancing a Task through its Workflow. */
export interface TaskExecution {
  id: string;
  taskId: string;
  fromState: WorkflowStateName;
  toState: WorkflowStateName;
  startedAt: string;
  finishedAt?: string;
}

/** A record of one agent acting on a Task/TaskExecution. */
export interface AgentRun {
  id: string;
  taskExecutionId: string;
  client: { id: string; type: string };
  startedAt: string;
  finishedAt?: string;
  summary?: string;
}

/** Any output produced by an AgentRun (code, design file, document, test report, ...). */
export interface Artifact {
  id: string;
  agentRunId: string;
  kind: string;
  uri: string;
}

/** A recorded decision made during a workflow, e.g. by a human approver. */
export interface Decision {
  id: string;
  taskExecutionId: string;
  decidedBy: string;
  outcome: 'approved' | 'rejected' | 'deferred';
  rationale?: string;
  decidedAt: string;
}

/** The result of validating an Artifact against its governing behavior/skill/policy. */
export interface Verification {
  id: string;
  artifactId: string;
  passed: boolean;
  details?: string;
}
