import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const backend = process.argv[2] ?? "starlingmonkey";
if (!new Set(["starlingmonkey", "quickjs"]).has(backend)) {
  throw new Error(`unknown TypeScript runtime backend: ${backend}`);
}

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");
const component = resolve(
  workspace,
  `target/typescript-runtime-spike/clock-typescript-${backend}.wasm`,
);
const baseArgs = ["test"];
if (process.env.FLOATILE_SPIKE_DISABLE_RUSTC_WRAPPER === "1") {
  baseArgs.unshift("--config", 'build.rustc-wrapper=""');
}
const env = { ...process.env, FLOATILE_TYPESCRIPT_CLOCK_WASM: component };

execFileSync(
  "cargo",
  [
    ...baseArgs,
    "-p",
    "floatile-runtime",
    "--test",
    "typescript_clock_spike",
    "typescript_clock",
    "--",
    "--ignored",
    "--nocapture",
    "--test-threads=1",
  ],
  { cwd: workspace, env, stdio: "inherit" },
);
execFileSync(
  "cargo",
  [
    ...baseArgs,
    "-p",
    "floatile-cli",
    "--test",
    "typescript_clock_spike",
    "--",
    "--ignored",
    "--nocapture",
  ],
  { cwd: workspace, env, stdio: "inherit" },
);
