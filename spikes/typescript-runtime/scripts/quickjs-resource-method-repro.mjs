import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");
const fixtureDir = resolve(spikeDir, "repro/quickjs-resource-method");
const output = resolve(
  workspace,
  "target/typescript-runtime-spike/quickjs-resource-method-repro.wasm",
);
const sourcePath = resolve(fixtureDir, "component.js");
mkdirSync(dirname(output), { recursive: true });

const candidateBin = process.env.FLOATILE_COMPONENTIZE_QJS_BIN;
if (candidateBin) {
  execFileSync(
    resolve(candidateBin),
    [
      "--wit",
      resolve(fixtureDir, "package.wit"),
      "--js",
      sourcePath,
      "--module-root",
      fixtureDir,
      "--output",
      output,
      "--world",
      "quickjs-repro",
      "--stub-wasi",
      "--opt-size",
      "--sync",
    ],
    { cwd: spikeDir, stdio: "inherit" },
  );
} else {
  const { componentize } = await import("componentize-qjs");
  const { component } = await componentize({
    witPath: resolve(fixtureDir, "package.wit"),
    jsSource: readFileSync(sourcePath, "utf8"),
    jsPath: sourcePath,
    moduleRoot: fixtureDir,
    world: "quickjs-repro",
    stubWasi: true,
    optSize: true,
    sync: true,
  });
  writeFileSync(output, component);
}

const cargoArgs = [
  "test",
  "-p",
  "floatile-runtime",
  "--test",
  "quickjs_resource_method_repro",
  ...(candidateBin
    ? ["quickjs_resource_method_arguments_reach_javascript_after_receiver_fix"]
    : []),
  "--",
  "--ignored",
  ...(!candidateBin
    ? [
        "--skip",
        "quickjs_resource_method_arguments_reach_javascript_after_receiver_fix",
      ]
    : []),
  "--nocapture",
  "--test-threads=1",
];
if (process.env.FLOATILE_SPIKE_DISABLE_RUSTC_WRAPPER === "1") {
  cargoArgs.unshift("--config", 'build.rustc-wrapper=""');
}
execFileSync("cargo", cargoArgs, {
  cwd: workspace,
  env: {
    ...process.env,
    FLOATILE_QUICKJS_REPRO_WASM: output,
  },
  stdio: "inherit",
});

console.log(
  JSON.stringify({
    backend: "componentize-qjs",
    version: candidateBin ? "candidate receiver fix" : "0.4.3",
    componentBytes: statSync(output).size,
    result: candidateBin
      ? "scalar and variant method arguments reach JavaScript"
      : "resource method arguments reproduce the adapter trap",
  }),
);
