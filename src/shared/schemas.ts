import { z } from 'zod';
import { RELATION_KINDS } from './types.js';

export const ClientInfoSchema = z.object({
  id: z.string().min(1),
  type: z.string().min(1),
});

export const ProjectFileSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  version: z.number().int().positive(),
  description: z.string().optional(),
});

export const SkillFrontmatterSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  version: z.string().min(1),
  scope: z.enum(['global', 'project']),
  description: z.string().min(1),
  tags: z.array(z.string()).optional(),
  dependsOn: z.array(z.string()).optional(),
});

export const BehaviorTransitionSchema = z.object({
  from: z.string().min(1),
  event: z.string().min(1),
  to: z.string().min(1),
});

export const BehaviorRuleSchema = z.object({
  id: z.string().min(1),
  description: z.string().min(1),
});

export const BehaviorFileSchema = z.object({
  id: z.string().min(1),
  type: z.string().min(1),
  version: z.number().int().positive(),
  states: z.array(z.string().min(1)).min(1),
  transitions: z.array(BehaviorTransitionSchema).default([]),
  rules: z.array(BehaviorRuleSchema).default([]),
});

export const EntityFileSchema = z
  .object({
    id: z.string().min(1),
    type: z.string().min(1),
    name: z.string().min(1),
  })
  .catchall(z.unknown());

export const RelationFileSchema = z.object({
  from: z.string().min(1),
  relation: z.enum(RELATION_KINDS),
  to: z.string().min(1),
});

export const PolicyFileSchema = z.object({
  client: z.string().min(1),
  permissions: z.array(z.string().min(1)).min(1),
});

export const ContextRequestSchema = z.object({
  projectId: z.string().min(1),
  task: z.string().min(1),
  client: ClientInfoSchema,
  requestedEntities: z.array(z.string()).optional(),
});

export const CapabilitySchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  description: z.string().min(1),
});
