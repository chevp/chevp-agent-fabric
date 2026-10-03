---
id: agent-nexus-mcp
name: Agent Nexus MCP Server
version: 1.0.0
scope: global
description: How to install, connect, and update the Agent Nexus MCP server in a project
tags:
  - mcp
  - tooling
  - agent-nexus
---

# Agent Nexus MCP Server

Rules for any agent that needs to install or maintain a connection to the
Agent Nexus MCP server (binary name `agent-nexus`) from a client project.

## Installing

Agent Nexus is a compiled Rust binary, not an npm package — never suggest
`npm install`/`npx` for it.

- Prefer downloading the matching prebuilt binary from the latest GitHub
  Release: `https://github.com/chevp/chevp-agent-fabric/releases/latest`.
- If no release asset matches the target platform, build from source:
  `git clone` the repo, then `cargo build --release`
  (binary at `target/release/agent-nexus`).
- Register the binary with the MCP client, pointing `AGENT_NEXUS_ROOT` at
  the Git repository that should act as the Control Plane's source of
  truth, e.g.:

  ```bash
  claude mcp add agent-nexus \
    --env AGENT_NEXUS_ROOT=/path/to/your/project \
    -- /path/to/agent-nexus
  ```

## Updating

Agent Nexus has no self-update mechanism — it is a local stdio process, not
a service that checks a registry on its own.

- A new version is a new `vX.Y.Z` git tag on `chevp/chevp-agent-fabric`,
  which triggers `.github/workflows/release.yml` and publishes a GitHub
  Release with the built binaries.
- To update: fetch the new release asset (or rebuild from the new tag),
  overwrite the existing binary in place, then restart the MCP client's
  connection to it. No client config changes are needed since the binary
  path does not change.
- The running server reports its version in the `initialize` response
  (`serverInfo.version`), which always matches `Cargo.toml`'s
  `[workspace.package].version` — and, on a release build, the tag it was
  cut from.

## Verifying a connection

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | ./agent-nexus
```

A healthy server responds to `initialize` with `serverInfo.name ==
"agent-nexus"` and to `tools/list` with the fixed domain tools plus any
registered concrete tools (see README section 9).
