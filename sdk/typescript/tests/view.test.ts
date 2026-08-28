import assert from "node:assert/strict";
import test from "node:test";

import { Column, Text, state } from "../dist/index.js";

test("generated builders emit canonical Floatile UI IR", () => {
  const view = Column({
    padding: 16,
    gap: 8,
    children: Text({ text: state("$.time"), style: "title" }),
  });
  assert.deepEqual(view, {
    type: "Column",
    props: { padding: 16, gap: 8 },
    children: [{ type: "Text", props: { text: { bind: "$.time" }, style: "title" } }],
  });
});

test("state bindings reject ambiguous paths", () => {
  assert.throws(() => state("time"), /must start/);
});
