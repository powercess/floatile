import { execFileSync } from "node:child_process";
import { readFileSync, statSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");
const backend = process.argv[2] ?? "starlingmonkey";
const componentPath = process.argv[3]
  ? resolve(process.argv[3])
  : resolve(
      workspace,
      `target/typescript-runtime-spike/clock-typescript-${backend}.wasm`,
    );
const uiPath = resolve(workspace, "target/typescript-runtime-spike/widget.ftui");
const jco = resolve(
  spikeDir,
  "node_modules/.bin",
  process.platform === "win32" ? "jco.CMD" : "jco",
);
const world = execFileSync(jco, ["wit", componentPath], {
  cwd: spikeDir,
  encoding: "utf8",
});

const imports = [...world.matchAll(/^\s*import\s+([^;]+);$/gm)].map(
  (match) => match[1],
);
const exports = [...world.matchAll(/^\s*export\s+([^;]+);$/gm)].map(
  (match) => match[1],
);
const unexpected = imports.filter(
  (name) => !name.startsWith("floatile:widget/"),
);
if (unexpected.length > 0) {
  throw new Error(`ambient imports are forbidden: ${unexpected.join(", ")}`);
}
if (imports.length !== 7) {
  throw new Error(`expected the seven WIT host interfaces, got ${imports.length}`);
}
if (!world.includes("export floatile:widget/widget-contract@1.0.0;")) {
  throw new Error("component does not export Floatile widget-contract@1.0.0");
}
const additionalExports = exports.filter(
  (name) => name !== "floatile:widget/widget-contract@1.0.0",
);
if (/^\s*import\s+wasi:/m.test(world)) {
  throw new Error("WASI imports are forbidden for the TypeScript adapter");
}

const componentBytes = statSync(componentPath).size;
if (componentBytes > 16 * 1024 * 1024) {
  throw new Error(`component exceeds the 16 MiB entry limit: ${componentBytes}`);
}
const ui = JSON.parse(readFileSync(uiPath, "utf8"));
if (ui.uiApiVersion !== "1.0.0") {
  throw new Error(`unexpected uiApiVersion: ${ui.uiApiVersion}`);
}

console.log(
  JSON.stringify({
    backend,
    componentBytes,
    ambientImports: 0,
    additionalExports,
    uiApiVersion: ui.uiApiVersion,
  }),
);
