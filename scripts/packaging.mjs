#!/usr/bin/env node
// Fill the Homebrew cask and winget manifest for a published GitHub release:
//   node scripts/packaging.mjs 0.4.0
// Downloads the release's .dmg and -setup.exe to compute their SHA-256, then writes
// packaging/out/rowhouse.rb and packaging/out/Novagen.Rowhouse.yaml.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const version = (process.argv[2] || "").replace(/^v/, "");
if (!/^\d+\.\d+\.\d+/.test(version)) {
  console.error("usage: node scripts/packaging.mjs <version>   e.g. 0.4.0");
  process.exit(1);
}
const base = `https://github.com/giankbr/rowhouse/releases/download/v${version}`;
async function sha(name) {
  const res = await fetch(`${base}/${name}`, { redirect: "follow" });
  if (!res.ok) throw new Error(`${name}: HTTP ${res.status} — is the release published?`);
  return createHash("sha256").update(Buffer.from(await res.arrayBuffer())).digest("hex");
}
const dmg = await sha(`Rowhouse_${version}_universal.dmg`);
const exe = await sha(`Rowhouse_${version}_x64-setup.exe`);
const out = join(root, "packaging", "out");
mkdirSync(out, { recursive: true });
const fill = (file, map) => Object.entries(map).reduce((t, [k, v]) => t.replaceAll(k, v), readFileSync(join(root, "packaging", file), "utf8"));
writeFileSync(join(out, "rowhouse.rb"), fill("homebrew/rowhouse.rb", { __VERSION__: version, __SHA256_DMG__: dmg }));
writeFileSync(join(out, "Novagen.Rowhouse.yaml"), fill("winget/Novagen.Rowhouse.yaml", { __VERSION__: version, __SHA256_EXE__: exe.toUpperCase() }));
console.log(`packaging/out/rowhouse.rb            sha256 ${dmg}`);
console.log(`packaging/out/Novagen.Rowhouse.yaml  sha256 ${exe}`);
