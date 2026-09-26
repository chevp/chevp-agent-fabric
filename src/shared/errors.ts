/** Base class for all Agent Nexus domain errors. */
export class NexusError extends Error {
  constructor(
    message: string,
    public readonly code: string,
  ) {
    super(message);
    this.name = new.target.name;
  }
}

export class NotFoundError extends NexusError {
  constructor(kind: string, id: string) {
    super(`${kind} "${id}" was not found`, 'NOT_FOUND');
  }
}

export class ValidationError extends NexusError {
  constructor(
    message: string,
    public readonly issues: unknown,
  ) {
    super(message, 'VALIDATION_ERROR');
  }
}

export class PermissionDeniedError extends NexusError {
  constructor(clientId: string, permission: string) {
    super(`Client "${clientId}" lacks permission "${permission}"`, 'PERMISSION_DENIED');
  }
}

export class VersionConflictError extends NexusError {
  constructor(message: string) {
    super(message, 'VERSION_CONFLICT');
  }
}
