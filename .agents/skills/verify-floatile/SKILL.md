---
name: verify-floatile
description: Validate Floatile changes and repository health against Rust quality gates, crate dependency rules, WIT and Permission Broker security invariants, package and persistence failure paths, cross-platform behavior, and P0 acceptance evidence. Use for pre-commit checks, code review, architecture or security audits, CI failures, dependency updates, release-readiness checks, and claims that a Floatile requirement or platform capability is complete.
---

# Verify Floatile

## Build the verification matrix

1. Read `AGENTS.md`, `CONTRIBUTING.md`, `docs/README.md`, the referenced requirements, and the diff or
   requested claim.
2. Separate compile-time evidence, automated behavior tests, real-platform tests, performance measurements,
   and security/adversarial tests. Do not let one substitute for another.
3. List affected targets: crates, host OSes/display protocols, `wasm32-wasip2`, WIT/API versions, migrations,
   permissions, package inputs, and documentation.

## Run the baseline

Use `--locked` and record failures exactly:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo check -p floatile-sdk --target wasm32-wasip2 --locked
cargo deny --locked check advisories bans sources
```

Treat `cargo deny --locked check licenses` as a release-blocking check until the licensing ADR is accepted;
do not weaken the policy to manufacture a pass.

## Apply focused checks

- For WIT changes, validate the component, regenerate both sides from the same source, and test supported and
  rejected API versions.
- For permissions, test allow, deny, scope, quota, redaction, resource exhaustion, and host survival. Look for
  any native capability path that bypasses the Broker.
- For packages and persistence, test malformed input, traversal/link/duplicate/size attacks, empty and old
  databases, transaction failure, and repeated migration.
- For platform/UI changes, compile all targets available in CI and perform the relevant real compositor,
  monitor, DPI, hot-plug, transparency, click-through, and renderer checks.
- For performance claims, use release builds, document hardware and sampling method, retain raw values, and
  compare them with the acceptance target without rounding a failure into a pass.
- For dependency changes, inspect `Cargo.lock`, enabled features, duplicates, source and license changes, MSRV,
  WASI compatibility, and three-host builds.

## Audit architecture and evidence

- Search for platform APIs outside `floatile-platform`, host dependencies in `floatile-sdk`, duplicated WIT,
  blocking work on the UI thread, broad `unsafe`, `unwrap` on untrusted paths, secrets in logs, and success
  flags without evidence.
- Confirm the docs required by the `AGENTS.md` change-coupling table changed with the implementation.
- When commits or a PR are in scope, confirm branch isolation, atomic buildable commits, mandatory message
  bodies and `Refs:`/`Tests:`/`Unverified:` entries, and absence of `Co-authored-by:` in any casing. Ordinary
  work must flow from task branches into `dev`; only release or hotfix flows may target `main`, and a hotfix
  must be synchronized back to `dev`.
- Mark unavailable environment checks as `未验证`; never copy expected platform-matrix symbols as results.

## Report

Lead with pass/fail and blocking findings. Then list commands and environment, focused evidence mapped to
requirement IDs, remaining risks, and unverified targets. Distinguish pre-existing gaps from regressions caused
by the current change.
