#!/usr/bin/env node
// One-time: turn on signed auto-updates.
//   node scripts/setup-updater.mjs
// 1. generates an updater key pair at ~/.tauri/rowhouse-updater.key (+ .pub)
//    (skipped if it already exists),
// 2. writes the public key + GitHub Releases endpoint into tauri.conf.json and
//    enables updater artifacts,
// 3. tells you which GitHub secrets to add. The private key never leaves your machine.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const conf = join(root, "apps", "desktop", "src-tauri", "tauri.conf.json");
const keyDir = join(homedir(), ".tauri");
const key = join(keyDir, "rowhouse-updater.key");
const repo = process.env.ROWHOUSE_REPO || "giankbr/rowhouse";

if (!existsSync(key)) {
  mkdirSync(keyDir, { recursive: true });
  console.log("Generating updater key (you'll be asked for a password — remember it)…");
  const args = ["--filter", "rowhouse-desktop", "exec", "tauri", "signer", "generate", "-w", key];
  // non-interactive use (CI / scripts): ROWHOUSE_UPDATER_PASSWORD=… node scripts/setup-updater.mjs
  if (process.env.ROWHOUSE_UPDATER_PASSWORD) args.push("--ci", "-p", process.env.ROWHOUSE_UPDATER_PASSWORD);
  execFileSync("pnpm", args, {
    cwd: root,
    stdio: "inherit",
  });
}
const pubkey = readFileSync(`${key}.pub`, "utf8").trim();

const c = JSON.parse(readFileSync(conf, "utf8"));
c.bundle.createUpdaterArtifacts = true;
c.plugins = c.plugins || {};
c.plugins.updater = {
  pubkey,
  endpoints: [`https://github.com/${repo}/releases/latest/download/latest.json`],
};
writeFileSync(conf, JSON.stringify(c, null, 2) + "\n");

console.log(`
✔ tauri.conf.json updated (public key + endpoint). Commit it.

Now add these repository secrets (GitHub → Settings → Secrets and variables → Actions):
  TAURI_SIGNING_PRIVATE_KEY           contents of ${key}
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD  the password you just chose

Keep ${key} safe and out of git: losing it means existing installs can't verify
future updates; leaking it lets someone else sign updates for your users.
`);
