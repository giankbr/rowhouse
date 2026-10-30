#!/usr/bin/env node
// Build rowhouse-mcp and place it where Tauri's `externalBin` expects it:
//   apps/desktop/src-tauri/binaries/rowhouse-mcp-<target-triple>[.exe]
// The `universal-apple-darwin` meta-target is not a real rustc target. Tauri's
// per-arch build step needs a sidecar for each arch triple, and its bundling step
// needs a lipo'd universal sidecar, so for it we emit all three.
// Usage: node scripts/sidecar.mjs [debug|release]   (default: release)
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const profile = process.argv[2] === "debug" ? "debug" : "release";
const host = execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/host: (\S+)/)[1];
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const dir = join(root, "apps", "desktop", "src-tauri", "binaries");
mkdirSync(dir, { recursive: true });

// Build rowhouse-mcp for one rust target triple; returns the built binary path.
// A build for the host triple reuses target/<profile>; anything else is a cross
// build with its own target/<triple>/<profile> dir. Pass forceTarget to keep the
// per-triple dir even when the target equals the host (needed for lipo inputs).
function build(target, forceTarget = false) {
  const ext = target.includes("windows") ? ".exe" : "";
  const perTarget = forceTarget || target !== host;
  const args = ["build", "-p", "rowhouse-mcp"];
  if (perTarget) args.push("--target", target);
  if (profile === "release") args.push("--release");
  execFileSync("cargo", args, { cwd: root, stdio: "inherit" });
  return join(root, "target", ...(perTarget ? [target] : []), profile, `rowhouse-mcp${ext}`);
}

if (triple === "universal-apple-darwin") {
  const slices = [];
  for (const t of ["aarch64-apple-darwin", "x86_64-apple-darwin"]) {
    const dest = join(dir, `rowhouse-mcp-${t}`);
    copyFileSync(build(t, true), dest);
    slices.push(dest);
    console.log(`sidecar ready: binaries/rowhouse-mcp-${t} (${profile})`);
  }
  execFileSync("lipo", ["-create", ...slices, "-output", join(dir, "rowhouse-mcp-universal-apple-darwin")], { stdio: "inherit" });
  console.log(`sidecar ready: binaries/rowhouse-mcp-universal-apple-darwin (${profile}, universal)`);
} else {
  const ext = triple.includes("windows") ? ".exe" : "";
  const src = build(triple);
  copyFileSync(src, join(dir, `rowhouse-mcp-${triple}${ext}`));
  console.log(`sidecar ready: binaries/rowhouse-mcp-${triple}${ext} (${profile})`);
}
