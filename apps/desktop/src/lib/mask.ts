// Masking personal data and secrets in the grid (screenshot mode, production
// connections). Same rules as rowhouse-core's mask.rs, which masks MCP results.
import type { Cell, ConnectionConfig } from "./api";

export type MaskKind = "email" | "phone" | "card" | "secret" | "id" | "address" | "name" | "birth";

/** Masking is on for a connection: its setting, or production by default. */
export const connMasks = (c?: ConnectionConfig | null) => !!c && (c.maskPii ?? c.env === "production");

function words(name: string): string[] {
  return name
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean);
}

export function columnKind(name: string): MaskKind | null {
  const w = words(name);
  const has = (x: string) => w.includes(x);
  const j = w.join("_");
  if (has("email") || has("mail") || j.includes("e_mail")) return "email";
  if (["phone", "mobile", "msisdn", "tel", "telephone", "whatsapp", "wa", "hp"].some(has)) return "phone";
  if (
    ["password", "passwd", "pwd", "secret", "token", "apikey", "salt", "otp", "pin", "credential", "credentials"].some(has) ||
    j.includes("api_key") ||
    j.includes("access_key") ||
    j.includes("private_key") ||
    (j.endsWith("_hash") && (j.includes("pass") || j.includes("pin")))
  )
    return "secret";
  if (["card", "cc", "pan", "iban", "cvv", "cvc"].some(has) || j.includes("account_number") || j.includes("bank_account")) return "card";
  if (["ssn", "nik", "ktp", "npwp", "passport", "nric", "nin", "sin"].some(has) || j.includes("national_id") || j.includes("tax_id") || j.includes("id_number")) return "id";
  if (["address", "street", "addr"].some(has)) return "address";
  if (["full_name", "first_name", "last_name", "surname", "given_name", "family_name", "customer_name", "contact_name"].includes(j)) return "name";
  if (has("dob") || j.includes("birth")) return "birth";
  return null;
}

const luhn = (d: number[]) => d.reverse().reduce((s, x, i) => s + (i % 2 ? (x * 2 > 9 ? x * 2 - 9 : x * 2) : x), 0) % 10 === 0;
const isEmail = (s: string) => /^[^\s@]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(s.trim());

export function valueKind(s: string): MaskKind | null {
  const t = s.trim();
  if (t.length < 6 || t.length > 4096) return null;
  if (isEmail(t)) return "email";
  if (t.startsWith("eyJ") && t.split(".").length === 3 && !t.includes(" ")) return "secret";
  if (/^(sk_live_|sk_test_|rk_live_|ghp_|gho_|github_pat_|xoxb-|xoxp-|AKIA|AIza|glpat-)\S{8,}$/.test(t)) return "secret";
  if (t.startsWith("-----BEGIN") && t.includes("PRIVATE KEY")) return "secret";
  if (/^[\d -]+$/.test(t)) {
    const d = [...t].filter((c) => /\d/.test(c)).map(Number);
    if (d.length >= 13 && d.length <= 19 && (t.match(/[ -]/g)?.length ?? 0) <= 4 && luhn([...d])) return "card";
  }
  return null;
}

function shape(s: string, keepEnd: number) {
  const c = [...s];
  const keep = Math.min(keepEnd, Math.floor(c.length / 3));
  return c.map((ch, i) => (i < c.length - keep && /[\p{L}\p{N}]/u.test(ch) ? "•" : ch)).join("");
}

export function maskText(kind: MaskKind, s: string): string {
  switch (kind) {
    case "email": {
      const [u, d] = s.trim().split("@");
      return d ? `${u[0] ?? "•"}•••@•••.${d.split(".").pop()}` : "•••";
    }
    case "phone":
    case "card":
    case "id":
      return shape(s, 4);
    case "secret":
      return "••••••••";
    case "birth":
      return /^\d{4}/.test(s) ? `${s.slice(0, 4)}-••-••` : "••••";
    default:
      return `${s.trim()[0] ?? "•"}•••`;
  }
}

function inline(s: string): string | null {
  if (!/\s/.test(s)) return null;
  let changed = false;
  const out = s
    .split(" ")
    .map((tok) => {
      const core = tok.replace(/^[,.;:!?()[\]<>"']+|[,.;:!?()[\]<>"']+$/g, "");
      const k = core && valueKind(core);
      if (!k) return tok;
      changed = true;
      return tok.replace(core, maskText(k, core));
    })
    .join(" ");
  return changed ? out : null;
}

/** Masked text for a cell, or null when it's fine to show. */
export function maskCell(kind: MaskKind | null, v: Cell | undefined): string | null {
  if (v === null || v === undefined) return null;
  if (typeof v === "string") {
    const k = kind ?? valueKind(v);
    return k ? maskText(k, v) : inline(v);
  }
  if (typeof v === "number") {
    const s = String(v);
    const k = kind ?? valueKind(s);
    return k ? maskText(k, s) : null;
  }
  if (typeof v === "object") {
    if (kind) return "••••";
    const s = JSON.stringify(v);
    let changed = false;
    const masked = JSON.stringify(v, (key, x) => {
      const k = key ? columnKind(key) : null;
      const m = typeof x === "string" || typeof x === "number" ? maskCell(k, x as Cell) : null;
      if (m !== null) {
        changed = true;
        return m;
      }
      return x;
    });
    return changed ? masked : s === masked ? null : masked;
  }
  return kind ? "••••" : null;
}
