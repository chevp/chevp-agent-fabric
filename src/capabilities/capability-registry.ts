import { NotFoundError } from '../shared/errors.js';
import type { Capability } from './capability.js';

/**
 * In-memory registry of capabilities (external systems/tools). V1 ships with
 * no capabilities registered by default; future work can populate this from
 * configuration or have external MCP servers register themselves at startup.
 */
export class CapabilityRegistry {
  private readonly capabilities = new Map<string, Capability>();

  register(capability: Capability): void {
    this.capabilities.set(capability.id, capability);
  }

  list(): Capability[] {
    return [...this.capabilities.values()];
  }

  get(id: string): Capability {
    const capability = this.capabilities.get(id);
    if (!capability) throw new NotFoundError('Capability', id);
    return capability;
  }
}
