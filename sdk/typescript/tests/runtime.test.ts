import assert from "node:assert/strict";
import test from "node:test";

import {
  createWidgetContract,
  defineWidget,
  WidgetError,
  type RuntimeWidgetEvent,
} from "../dist/index.js";

interface State {
  readonly count: number;
}

test("contract adapter commits the local State mirror only after host acceptance", () => {
  const patches: string[] = [];
  const widget = defineWidget<State, number>({
    state: { count: 0 },
    view: () => ({ type: "Text" }),
    event: (amount, context) => {
      assert.deepEqual(context.config, { mode: "test" });
      context.state.update({ count: amount });
    },
  });
  const contract = createWidgetContract(
    widget,
    { updateState: (patch) => patches.push(patch) },
    { decodeEvent: (event) => event.tag === "timer" ? event.val : undefined },
  );
  const instance = new contract.WidgetInstance({ configJson: '{"mode":"test"}', initialStateJson: '{"count":0}' });
  instance.handleEvent({ tag: "timer", val: 2 });
  assert.deepEqual(patches, ['{"count":2}']);
});

test("contract adapter lowers author and unknown lifecycle errors", () => {
  const event = { tag: "resume" } satisfies RuntimeWidgetEvent;
  const contract = createWidgetContract(
    defineWidget<State, RuntimeWidgetEvent>({
      state: { count: 0 },
      view: () => ({ type: "Text" }),
      event: () => { throw WidgetError.rejected("busy"); },
    }),
    { updateState: () => undefined },
    { decodeEvent: (raw) => raw },
  );
  const instance = new contract.WidgetInstance({ configJson: "{}", initialStateJson: '{"count":0}' });
  assert.throws(() => instance.handleEvent(event), (error) => {
    assert.deepEqual(error, { tag: "rejected", val: "busy" });
    return true;
  });

  const unknown = createWidgetContract(
    defineWidget<State>({
      state: { count: 0 },
      view: () => ({ type: "Text" }),
      start: () => { throw new Error("secret"); },
    }),
    { updateState: () => undefined },
    { decodeEvent: () => undefined },
  );
  const unknownInstance = new unknown.WidgetInstance({ configJson: "{}", initialStateJson: '{"count":0}' });
  assert.throws(() => unknownInstance.start(), (error) => {
    assert.deepEqual(error, { tag: "internal" });
    return true;
  });
});
