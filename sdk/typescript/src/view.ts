export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { readonly [key: string]: JsonValue };

export interface StateBinding {
  readonly bind: string;
}

export interface ItemBinding {
  readonly item: string;
}

export type Binding = StateBinding | ItemBinding;
export type Bindable<T extends JsonValue> = T | Binding;

export interface EmittedEvent {
  readonly emit: string;
  readonly payload?: JsonValue;
}

export interface View {
  readonly type: string;
  readonly props?: Readonly<Record<string, JsonValue | Binding>>;
  readonly children?: readonly View[];
  readonly events?: Readonly<Record<string, EmittedEvent>>;
}

export function state(path: string): StateBinding {
  if (!path.startsWith("$.")) throw new TypeError("state binding must start with $.");
  return { bind: path };
}

export function item(field: string): ItemBinding {
  if (field.length === 0) throw new TypeError("item binding must not be empty");
  return { item: field };
}

export function component<T extends object>(type: string, input: T): View {
  const { children: childInput, ...props } = input as T & { readonly children?: View | readonly View[] };
  const children = childInput === undefined
    ? []
    : Array.isArray(childInput) ? childInput : [childInput];
  const filtered = Object.fromEntries(
    Object.entries(props).filter(([, value]) => value !== undefined),
  ) as Record<string, JsonValue | Binding>;
  return {
    type,
    ...(Object.keys(filtered).length === 0 ? {} : { props: filtered }),
    ...(children.length === 0 ? {} : { children }),
  };
}
