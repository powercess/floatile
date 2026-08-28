import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const backend = process.argv[2] ?? "starlingmonkey";
if (!new Set(["starlingmonkey", "quickjs"]).has(backend)) {
  throw new Error(`unknown TypeScript runtime backend: ${backend}`);
}

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");
const target = resolve(workspace, "target/typescript-runtime-spike");
const output = resolve(target, `clock-typescript-${backend}.wasm`);
const executable = (name) =>
  resolve(
    spikeDir,
    "node_modules/.bin",
    process.platform === "win32" ? `${name}.CMD` : name,
  );
const pnpm = process.platform === "win32" ? "pnpm.cmd" : "pnpm";

execFileSync(pnpm, ["--dir", "sdk/typescript", "build"], {
  cwd: workspace,
  stdio: "inherit",
});

execFileSync(process.execPath, ["scripts/prepare.mjs"], {
  cwd: spikeDir,
  stdio: "inherit",
});
execFileSync(
  executable("jco"),
  [
    "guest-types",
    "../../wit",
    "--world-name",
    "floatile-widget",
    "--out-dir",
    "generated",
    "--strict",
    "--quiet",
  ],
  { cwd: spikeDir, stdio: "inherit" },
);

if (backend === "starlingmonkey") {
  execFileSync(executable("tsc"), ["--noEmit"], {
    cwd: spikeDir,
    stdio: "inherit",
  });
  execFileSync(
    executable("jco"),
    [
      "componentize",
      "src/clock.ts",
      "--wit",
      "../../wit",
      "--world-name",
      "floatile-widget",
      "--disable",
      "all",
      "--out",
      output,
    ],
    { cwd: spikeDir, stdio: "inherit" },
  );
} else if (process.env.FLOATILE_COMPONENTIZE_QJS_BIN) {
  execFileSync(
    resolve(process.env.FLOATILE_COMPONENTIZE_QJS_BIN),
    [
      "--wit",
      resolve(workspace, "wit"),
      "--js",
      resolve(spikeDir, "src/clock-quickjs.js"),
      "--module-root",
      workspace,
      "--output",
      output,
      "--world",
      "floatile-widget",
      "--stub-wasi",
      "--opt-size",
      "--sync",
    ],
    { cwd: spikeDir, stdio: "inherit" },
  );
} else {
  const { componentize } = await import("componentize-qjs");
  const sourcePath = resolve(spikeDir, "src/clock-quickjs.js");
  const { component } = await componentize({
    witPath: resolve(workspace, "wit"),
    jsSource: readFileSync(sourcePath, "utf8"),
    jsPath: sourcePath,
    moduleRoot: workspace,
    world: "floatile-widget",
    stubWasi: true,
    optSize: true,
    sync: true,
  });
  writeFileSync(output, component);
}

execFileSync(process.execPath, ["scripts/build-ui.mjs"], {
  cwd: spikeDir,
  stdio: "inherit",
});
execFileSync(process.execPath, ["scripts/verify-component.mjs", backend, output], {
  cwd: spikeDir,
  stdio: "inherit",
});
