import type { JsonObject, JsonValue, View } from "./view.js";

export type WidgetErrorKind = "invalid-input" | "rejected" | "internal";

export class WidgetError extends Error {
  readonly kind: WidgetErrorKind;

  private constructor(kind: WidgetErrorKind, message?: string) {
    super(message ?? "internal widget error");
    this.name = "WidgetError";
    this.kind = kind;
  }

  static invalidInput(message: string): WidgetError {
    return new WidgetError("invalid-input", message);
  }

  static rejected(message: string): WidgetError {
    return new WidgetError("rejected", message);
  }

  static internal(): WidgetError {
    return new WidgetError("internal");
  }
}

export type WitWidgetError =
  | { readonly tag: "invalid-input"; readonly val: string }
  | { readonly tag: "rejected"; readonly val: string }
  | { readonly tag: "internal" };

export function lowerWidgetError(error: unknown): WitWidgetError {
  if (!(error instanceof WidgetError)) return { tag: "internal" };
  if (error.kind === "internal") return { tag: "internal" };
  return { tag: error.kind, val: error.message };
}

export interface StateContext<State extends JsonObject> {
  update(patch: Partial<State>): void;
}

export interface WidgetContext<State extends JsonObject> {
  readonly state: StateContext<State>;
  readonly config: JsonValue;
}

export interface WidgetDefinition<State extends JsonObject, Event> {
  readonly state: State;
  readonly view: (state: Readonly<State>) => View;
  readonly start?: (context: WidgetContext<State>) => void;
  readonly event?: (event: Event, context: WidgetContext<State>) => void;
  readonly stop?: (context: WidgetContext<State>) => void;
}

export function defineWidget<State extends JsonObject, Event = JsonValue>(
  definition: WidgetDefinition<State, Event>,
): WidgetDefinition<State, Event> {
  return Object.freeze(definition);
}
