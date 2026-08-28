import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { WidgetError, lowerWidgetError } from "../src/index.ts";

interface LifecycleVector {
  readonly id: string;
  readonly callback: "start" | "event";
  readonly guestError: "invalid-input" | "rejected" | "internal";
  readonly message: string | null;
  readonly expectedHostOutcome: "rejected";
}

interface LifecycleSuite {
  readonly schemaVersion: 1;
  readonly engineApiVersion: string;
  readonly vectors: readonly LifecycleVector[];
}

const lifecycle = JSON.parse(
  await readFile(new URL("../../../conformance/sdk-lifecycle-v1.json", import.meta.url), "utf8"),
) as LifecycleSuite;

interface SecuritySuite {
  readonly schemaVersion: 1;
  readonly engineApiVersion: string;
  readonly vectors: readonly {
    readonly id: string;
    readonly trigger: "start" | "event";
    readonly expectedHostOutcome: string;
    readonly hostSurvives: true;
  }[];
}

const security = JSON.parse(
  await readFile(new URL("../../../conformance/sdk-security-v1.json", import.meta.url), "utf8"),
) as SecuritySuite;

test("TypeScript errors lower to every shared lifecycle vector", () => {
  assert.equal(lifecycle.schemaVersion, 1);
  assert.equal(lifecycle.engineApiVersion, "1.2.0");
  assert.equal(lifecycle.vectors.length, 6);
  for (const vector of lifecycle.vectors) {
    const error = vector.guestError === "invalid-input"
      ? WidgetError.invalidInput(vector.message ?? "")
      : vector.guestError === "rejected"
        ? WidgetError.rejected(vector.message ?? "")
        : WidgetError.internal();
    const lowered = lowerWidgetError(error);
    assert.equal(lowered.tag, vector.guestError, vector.id);
    if ("val" in lowered) assert.equal(lowered.val, vector.message, vector.id);
    assert.equal(vector.expectedHostOutcome, "rejected");
  }
});

test("unknown JavaScript exceptions do not leak through the WIT boundary", () => {
  assert.deepEqual(lowerWidgetError(new Error("secret host path")), { tag: "internal" });
});

test("TypeScript adapter inherits every shared host-survival requirement", () => {
  assert.equal(security.schemaVersion, 1);
  assert.equal(security.engineApiVersion, lifecycle.engineApiVersion);
  assert.deepEqual(
    security.vectors.map((vector) => vector.id),
    ["broker-deny", "invalid-state-patch", "fuel-exhaustion", "wall-clock-timeout", "memory-limit"],
  );
  assert.ok(security.vectors.every((vector) => vector.hostSurvives));
});
