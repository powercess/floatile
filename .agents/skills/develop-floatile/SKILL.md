---
name: develop-floatile
description: Implement and modify the Floatile Rust workspace while preserving its crate boundaries, cross-platform degradation model, WIT single-source contract, Permission Broker security boundary, and P0 evidence requirements. Use for Floatile features, bug fixes, refactors, dependencies, platform/window behavior, plugin runtime or SDK work, manifests, persistence, CI, and normative documentation changes in this repository.
---

# Develop Floatile

## Establish scope

1. Read `AGENTS.md`, `CONTRIBUTING.md`, and `docs/README.md` from the repository root. Treat the pinned
   branch and commit rules in `AGENTS.md` and the complete Git policy in `CONTRIBUTING.md` as mandatory.
2. Map the request to requirement IDs in `docs/product/requirements.md` and acceptance items in
   `docs/architecture/p0-acceptance.md`.
3. Read only the task's authoritative documents from the docs index. Inspect the affected crate manifests,
   source, tests, and current worktree before editing.
4. State any assumption that changes platform support, plugin compatibility, security, persistence, licensing,
   or product scope. Require an ADR for an irreversible choice.

## Select the owning layer

- Put pure domain types and validation in `floatile-core`; keep it free of I/O and platform/runtime concerns.
- Put OS and window-system behavior, capability probes, and platform `unsafe` in `floatile-platform`.
- Put WIT host bindings in `floatile-plugin-api`, untrusted execution in `floatile-runtime`, and mediated host
  capability implementations in `floatile-services`.
- Put SQLite and migrations in `floatile-store`, host composition and UI state in `floatile-shell`, guest-only
  ergonomics in `floatile-sdk`, and packaging/validation commands in `floatile-cli`.
- Reject a shortcut that reverses the dependency graph or duplicates a cross-layer type to hide coupling.

## Preserve mandatory boundaries

- Route every plugin host capability through a deny-by-default `PermissionBroker`; couple authorization,
  scope/quota enforcement, execution, and redacted audit behavior.
- Treat WIT under `wit/` as the only host/guest contract source. Couple WIT changes to both bindings, runtime
  adapters, API versioning, docs, and contract tests.
- Treat manifest, archive paths, Slint, WASM, configuration, and plugin arguments as untrusted input.
- Keep the Slint event loop non-blocking. Define bounded queues, timeout, cancellation, and shutdown for
  background work.
- Expose platform capability and degradation reasons instead of guessing by OS name or silently succeeding.

## Implement a vertical slice

1. Add a failing behavior/security test or a reproducible manual case.
2. Implement the smallest complete path through the owning layers, including error and degradation paths.
3. Use workspace dependencies and inherited lints. Follow `docs/development/coding-standards.md`.
4. Update every coupled contract identified in `AGENTS.md`; never pre-fill platform or acceptance success.
5. Remove cargo-template placeholders only when replacing them with real scoped behavior.

## Respect Git collaboration boundaries

- Do not create or switch branches, stage, commit, push, rebase, merge, or rewrite history unless the user
  explicitly authorizes that action.
- Treat `dev` as the integration base for ordinary work and `main` as release-only. Create normal task branches
  from current `dev`; only release and hotfix flows described by `CONTRIBUTING.md` may target `main`.
- Before and after any authorized Git mutation, inspect the worktree and relevant diff. Preserve unrelated and
  concurrent changes, and stage only files belonging to the current task.
- Commit only an independently reviewable, buildable, tested step that satisfies `CONTRIBUTING.md`. Continue
  developing when the change is partial, failing, missing coupled artifacts, or would require a WIP message.
- Every authorized commit must have the required body and `Refs:`, `Tests:`, and `Unverified:` entries. Never
  add `Co-authored-by:` in any casing.

## Verify and hand off

Run targeted checks while iterating, then invoke the `$verify-floatile` workflow at
`.agents/skills/verify-floatile` for the final validation. Report requirement IDs, changed boundaries, actual
commands/results, environment-specific evidence, and explicit unverified items.
