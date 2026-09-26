---
id: code-style
name: Code Style
version: 1.0.0
scope: global
description: General coding-agent conventions applicable across all projects
tags:
  - code
  - style
  - engineering
---

# Code Style

General conventions for any coding agent (Copilot, an IDE agent, ...)
working in any Agent Nexus project, unless a project skill overrides it.

## Rules

- Prefer small, composable functions with explicit types over large,
  implicit ones.
- Do not introduce a new dependency for something the standard library
  already does well.
- Match the existing formatting/linting configuration of the project you are
  editing rather than imposing a different style.
