import { readFile, writeFile } from "node:fs/promises";

const [input, output, mode] = process.argv.slice(2);
if (!input || !output) {
  throw new Error("usage: generate-components.mjs <registry.json> <components.ts> [--check]");
}

const registry = JSON.parse(await readFile(input, "utf8"));
if (registry.schemaVersion !== 1 || typeof registry.uiApiVersion !== "string") {
  throw new Error("unsupported UI registry contract");
}

const scalar = (types) => [...new Set(types.map((type) => {
  if (type === "string") return "string";
  if (type === "number" || type === "integer") return "number";
  if (type === "boolean") return "boolean";
  if (type === "null") return "null";
  return "JsonValue";
}))].join(" | ");

let source = `// Generated from floatile-ui-schema ${registry.uiApiVersion}; do not edit.\n`;
source += 'import { component } from "../view.js";\n';
source += 'import type { Bindable, JsonValue, View } from "../view.js";\n\n';
source += `export const UI_API_VERSION = ${JSON.stringify(registry.uiApiVersion)} as const;\n\n`;
for (const spec of registry.components) {
  if (spec.kind !== "element") continue;
  const interfaceName = `${spec.name}Props`;
  source += `export interface ${interfaceName} {\n`;
  for (const prop of spec.props) {
    const base = scalar(prop.types);
    const type = prop.allowBinding ? `Bindable<${base}>` : base;
    source += `  readonly ${prop.name}${prop.optional ? "?" : ""}: ${type};\n`;
  }
  if (spec.children === "forbidden") {
    source += "  readonly children?: never;\n";
  } else if (spec.children === "one") {
    source += "  readonly children: View;\n";
  } else {
    source += "  readonly children?: View | readonly View[];\n";
  }
  source += "}\n";
  source += `export function ${spec.name}(props: ${interfaceName}): View {\n`;
  source += `  return component(${JSON.stringify(spec.name)}, props);\n`;
  source += "}\n\n";
}

if (mode === "--check") {
  const existing = await readFile(output, "utf8");
  if (existing !== source) throw new Error("generated TypeScript components are stale");
} else {
  await writeFile(output, source);
}
