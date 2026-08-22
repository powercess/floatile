import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");
const cargoPrefix =
  process.env.FLOATILE_SPIKE_DISABLE_RUSTC_WRAPPER === "1"
    ? ["--config", 'build.rustc-wrapper=""']
    : [];
const ftui = execFileSync(
  "cargo",
  [
    ...cargoPrefix,
    "run",
    "--quiet",
    "--bin",
    "build_ftui",
    "--features",
    "build-host",
    "--manifest-path",
    "plugins/clock-wasm/Cargo.toml",
  ],
  { cwd: workspace, encoding: "utf8" },
);

JSON.parse(ftui);
writeFileSync(
  resolve(workspace, "target/typescript-runtime-spike/widget.ftui"),
  ftui,
  "utf8",
);
