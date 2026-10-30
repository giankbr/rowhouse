#!/usr/bin/env node
// Build rowhouse-mcp and place it where Tauri's `externalBin` expects it:
//   apps/desktop/src-tauri/binaries/rowhouse-mcp-<target-triple>[.exe]
// Usage: node scripts/sidecar.mjs [debug|release]   (default: release)
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv[2] === "debug" ? "debug" : "release";
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/host: (\S+)/)[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const ext = triple.includes("windows") ? ".exe" : "";
// cross builds need --target (own target dir); native builds reuse target/<profile>
const cross = triple !== host;

const args = ["build", "-p", "rowhouse-mcp"];
if (cross) args.push("--target", triple);
if (profile === "release") args.push("--release");
execFileSync("cargo", args, { cwd: root, stdio: "inherit" });

const src = join(root, "target", ...(cross ? [triple] : []), profile, `rowhouse-mcp${ext}`);
const dir = join(root, "apps", "desktop", "src-tauri", "binaries");
mkdirSync(dir, { recursive: true });
copyFileSync(src, join(dir, `rowhouse-mcp-${triple}${ext}`));
console.log(`sidecar ready: binaries/rowhouse-mcp-${triple}${ext} (${profile})`);
