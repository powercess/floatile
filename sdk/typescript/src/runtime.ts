import type { JsonObject, JsonValue } from "./view.js";
import {
  lowerWidgetError,
  type WidgetContext,
  type WidgetDefinition,
} from "./widget.js";

export interface RuntimeWidgetInit {
  readonly configJson: string;
  readonly initialStateJson: string;
}

export type RuntimeWidgetEvent =
  | { readonly tag: "ui"; readonly val: { readonly name: string; readonly payloadJson: string } }
  | { readonly tag: "timer"; readonly val: number }
  | { readonly tag: "mode-changed"; readonly val: "edit" | "show" }
  | { readonly tag: "config-changed"; readonly val: string }
  | { readonly tag: "theme-changed"; readonly val: string }
  | { readonly tag: "operation-completed"; readonly val: JsonValue }
  | { readonly tag: "suspend" }
  | { readonly tag: "resume" };

export interface RuntimeHost {
  updateState(patchJson: string): void;
}

export interface WidgetContractAdapter<Event> {
  decodeEvent(event: RuntimeWidgetEvent): Event | undefined;
}

export interface WidgetContractConstructor {
  new(init: RuntimeWidgetInit): {
    start(): void;
    handleEvent(event: RuntimeWidgetEvent): void;
    stop(): void;
  };
}

export interface WidgetContract {
  readonly WidgetInstance: WidgetContractConstructor;
}

function parseState<State extends JsonObject>(json: string): State {
  const value: unknown = JSON.parse(json);
  if (value === null || Array.isArray(value) || typeof value !== "object") {
    throw new TypeError("initial State must be a JSON object");
  }
  return value as State;
}

function invoke(callback: () => void): void {
  try {
    callback();
  } catch (error) {
    throw lowerWidgetError(error);
  }
}

export function createWidgetContract<State extends JsonObject, Event>(
  definition: WidgetDefinition<State, Event>,
  host: RuntimeHost,
  adapter: WidgetContractAdapter<Event>,
): WidgetContract {
  return {
    WidgetInstance: class {
      readonly #context: WidgetContext<State>;
      #state: State;

      constructor(init: RuntimeWidgetInit) {
        this.#state = parseState<State>(init.initialStateJson);
        const config: unknown = JSON.parse(init.configJson);
        this.#context = {
          config: config as JsonValue,
          state: {
            update: (patch) => {
              host.updateState(JSON.stringify(patch));
              this.#state = { ...this.#state, ...patch };
            },
          },
        };
      }

      start(): void {
        if (definition.start) invoke(() => definition.start?.(this.#context));
      }

      handleEvent(event: RuntimeWidgetEvent): void {
        if (!definition.event) return;
        const decoded = adapter.decodeEvent(event);
        if (decoded !== undefined) {
          invoke(() => definition.event?.(decoded, this.#context));
        }
      }

      stop(): void {
        if (definition.stop) invoke(() => definition.stop?.(this.#context));
      }
    },
  };
}
