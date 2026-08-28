# `@floatile/sdk` TypeScript author surface

> Internal PP-M7 implementation. The package remains `private` until a TypeScript Component runtime passes
> ADR, resource, isolation, cross-platform, and licensing gates.

The build-time UI API is generated from `floatile-ui-schema` rather than a handwritten component list:

```text
floatile schema ui target/floatile-ui-registry.json
node sdk/typescript/scripts/generate-components.mjs \
  target/floatile-ui-registry.json sdk/typescript/src/generated/components.ts --check
```

`pnpm typecheck` verifies generated props and author-facing bindings. `pnpm test` checks that builders emit the
same canonical `widget.ftui` component shape consumed by the Rust SDK and host validator. This package does not
select or bundle a JavaScript runtime and is not authorized for registry publication.
