// Generated from floatile-ui-schema 1.6.0; do not edit.
import { component } from "../view.ts";
import type { Bindable, JsonValue, View } from "../view.ts";

export const UI_API_VERSION = "1.6.0" as const;

export interface RowProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly children?: View | readonly View[];
}
export function Row(props: RowProps): View {
  return component("Row", props);
}

export interface ColumnProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly children?: View | readonly View[];
}
export function Column(props: ColumnProps): View {
  return component("Column", props);
}

export interface StackProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly children?: View | readonly View[];
}
export function Stack(props: StackProps): View {
  return component("Stack", props);
}

export interface GridProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly columns?: number;
  readonly children?: View | readonly View[];
}
export function Grid(props: GridProps): View {
  return component("Grid", props);
}

export interface ScrollProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly children?: View | readonly View[];
}
export function Scroll(props: ScrollProps): View {
  return component("Scroll", props);
}

export interface ResponsiveProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly breakpoint: number;
  readonly children?: View | readonly View[];
}
export function Responsive(props: ResponsiveProps): View {
  return component("Responsive", props);
}

export interface TextProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly text: Bindable<string>;
  readonly style?: string;
  readonly colorToken?: string;
  readonly children?: never;
}
export function Text(props: TextProps): View {
  return component("Text", props);
}

export interface IconProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly name: Bindable<string>;
  readonly size?: number;
  readonly children?: never;
}
export function Icon(props: IconProps): View {
  return component("Icon", props);
}

export interface ImageProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly asset: string;
  readonly width?: number;
  readonly height?: number;
  readonly children?: never;
}
export function Image(props: ImageProps): View {
  return component("Image", props);
}

export interface ButtonProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly label: Bindable<string>;
  readonly children?: never;
}
export function Button(props: ButtonProps): View {
  return component("Button", props);
}

export interface ToggleProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly checked: Bindable<boolean>;
  readonly accessibilityLabel?: Bindable<string>;
  readonly children?: never;
}
export function Toggle(props: ToggleProps): View {
  return component("Toggle", props);
}

export interface ProgressProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly value: Bindable<number>;
  readonly accessibilityLabel?: Bindable<string>;
  readonly children?: never;
}
export function Progress(props: ProgressProps): View {
  return component("Progress", props);
}

export interface BadgeProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly label: Bindable<string>;
  readonly tone?: string;
  readonly children?: never;
}
export function Badge(props: BadgeProps): View {
  return component("Badge", props);
}

export interface GaugeProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly value: Bindable<number>;
  readonly accessibilityLabel?: Bindable<string>;
  readonly children?: never;
}
export function Gauge(props: GaugeProps): View {
  return component("Gauge", props);
}

export interface ListProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly items?: Bindable<JsonValue>;
  readonly children?: View | readonly View[];
}
export function List(props: ListProps): View {
  return component("List", props);
}

export interface SparklineProps {
  readonly padding?: number;
  readonly gap?: number;
  readonly width?: number;
  readonly height?: number;
  readonly opacity?: number;
  readonly radius?: number;
  readonly color?: string;
  readonly border?: string;
  readonly values: Bindable<JsonValue>;
  readonly label: Bindable<string>;
  readonly tone?: string;
  readonly children?: never;
}
export function Sparkline(props: SparklineProps): View {
  return component("Sparkline", props);
}

