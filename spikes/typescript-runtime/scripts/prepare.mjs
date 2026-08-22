import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const spikeDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspace = resolve(spikeDir, "../..");

mkdirSync(resolve(spikeDir, "generated"), { recursive: true });
mkdirSync(resolve(workspace, "target/typescript-runtime-spike"), { recursive: true });
