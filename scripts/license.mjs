#!/usr/bin/env node
// Rowhouse Pro license keys (Ed25519, verified offline by the app).
//
//   node scripts/license.mjs keygen
//       → creates ~/.rowhouse-license/private.pem (keep it secret, back it up)
//         and prints the public key for the ROWHOUSE_LICENSE_PUBKEY build secret.
//   node scripts/license.mjs issue --email buyer@example.com [--plan pro|team] [--seats 1] [--years 1]
//       → prints a key to send to the customer.
//   node scripts/license.mjs verify <key>
//
// The private key never goes into the repo or CI.
import { createPrivateKey, createPublicKey, generateKeyPairSync, randomUUID, sign, verify } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const dir = process.env.ROWHOUSE_LICENSE_DIR || join(homedir(), ".rowhouse-license");
const keyFile = join(dir, "private.pem");
const [cmd, ...rest] = process.argv.slice(2);
const arg = (name, def) => {
  const i = rest.indexOf(`--${name}`);
  return i >= 0 ? rest[i + 1] : def;
};

function privateKey() {
  if (!existsSync(keyFile)) {
    console.error(`No signing key at ${keyFile} — run: node scripts/license.mjs keygen`);
    process.exit(1);
  }
  return createPrivateKey(readFileSync(keyFile));
}
const rawPublic = (key) => createPublicKey(key).export({ format: "jwk" }).x; // base64url, 32 bytes

if (cmd === "keygen") {
  if (existsSync(keyFile)) {
    console.error(`${keyFile} already exists — not overwriting it (every issued key depends on it).`);
    process.exit(1);
  }
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const { privateKey: pk } = generateKeyPairSync("ed25519");
  writeFileSync(keyFile, pk.export({ format: "pem", type: "pkcs8" }), { mode: 0o600 });
  console.log(`Signing key written to ${keyFile} — back it up somewhere safe.\n`);
  console.log("Add this GitHub Actions secret (repo → Settings → Secrets → Actions):\n");
  console.log(`  ROWHOUSE_LICENSE_PUBKEY = ${rawPublic(pk)}\n`);
  console.log("Release builds with that secret ask for a license for Pro features; builds without it unlock everything.");
} else if (cmd === "issue") {
  const email = arg("email");
  if (!email) {
    console.error("usage: node scripts/license.mjs issue --email buyer@example.com [--plan pro|team] [--seats 1] [--years 1]");
    process.exit(1);
  }
  const now = Math.floor(Date.now() / 1000);
  const years = Number(arg("years", "1"));
  const payload = Buffer.from(
    JSON.stringify({
      email,
      plan: arg("plan", "pro"),
      seats: Number(arg("seats", "1")),
      issued: now,
      updatesUntil: now + Math.round(years * 365.25 * 86400),
      id: randomUUID(),
    }),
  );
  const sig = sign(null, payload, privateKey());
  console.log(`RH1-${payload.toString("base64url")}.${sig.toString("base64url")}`);
} else if (cmd === "verify") {
  const key = (rest[0] || "").trim();
  const [payload, sig] = key.replace(/^RH1-/, "").split(".");
  const ok = verify(null, Buffer.from(payload, "base64url"), createPublicKey(privateKey()), Buffer.from(sig, "base64url"));
  console.log(ok ? "valid" : "INVALID");
  if (ok) console.log(JSON.parse(Buffer.from(payload, "base64url").toString()));
  process.exit(ok ? 0 : 1);
} else {
  console.log("usage: node scripts/license.mjs keygen | issue --email … | verify <key>");
}
