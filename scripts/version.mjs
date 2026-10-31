#!/usr/bin/env node
// The version lives in three files; keep them in step.
//
//   node scripts/version.mjs              print the versions (fails if they differ)
//   node scripts/version.mjs 0.7.0        set all three
//   node scripts/version.mjs --check v0.7.0   fail unless all three equal the tag (CI)
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const files = {
  cargo: `${root}Cargo.toml`,
  tauri: `${root}apps/desktop/src-tauri/tauri.conf.json`,
  pkg: `${root}apps/desktop/package.json`,
};

function read() {
  const cargo = /\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/s.exec(readFileSync(files.cargo, "utf8"))?.[1];
  const tauri = JSON.parse(readFileSync(files.tauri, "utf8")).version;
  const pkg = JSON.parse(readFileSync(files.pkg, "utf8")).version;
  return { cargo, tauri, pkg };
}

function write(v) {
  const c = readFileSync(files.cargo, "utf8");
  const next = c.replace(/(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/s, `$1${v}$2`);
  if (next === c && read().cargo !== v) throw new Error("couldn't find [workspace.package] version in Cargo.toml");
  writeFileSync(files.cargo, next);
  for (const f of [files.tauri, files.pkg]) {
    const j = JSON.parse(readFileSync(f, "utf8"));
    j.version = v;
    writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
  }
}

const [arg, tag] = process.argv.slice(2);
const semver = /^\d+\.\d+\.\d+(-[\w.]+)?$/;
if (arg === "--check") {
  const want = (tag ?? "").replace(/^v/, "");
  const v = read();
  const bad = Object.entries(v).filter(([, x]) => x !== want);
  if (bad.length) {
    console.error(`tag ${tag} doesn't match: ${bad.map(([k, x]) => `${k}=${x}`).join(", ")}\nrun: node scripts/version.mjs ${want}`);
    process.exit(1);
  }
  console.log(`version ${want} ✓`);
} else if (arg) {
  if (!semver.test(arg)) {
    console.error(`not a version: ${arg} (expected e.g. 0.7.0)`);
    process.exit(1);
  }
  write(arg);
  console.log(`set ${arg} in Cargo.toml, tauri.conf.json and package.json — then: git commit -am "chore(release): v${arg}" && git tag v${arg}`);
} else {
  const v = read();
  console.log(v);
  if (new Set(Object.values(v)).size !== 1) {
    console.error("versions differ — run: node scripts/version.mjs <version>");
    process.exit(1);
  }
}
