// End-to-end smoke tests against the in-browser mock (no Tauri, no database).
// Run: pnpm --filter rowhouse-desktop e2e   (starts vite on a free port)
import { after, before, describe, test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { chromium } from "playwright";

const PORT = Number(process.env.E2E_PORT ?? 1430);
const URL = process.env.E2E_URL ?? `http://localhost:${PORT}/`;

let server;
let browser;
let page;
const errors = [];

async function waitFor(url, ms = 60_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    try {
      if ((await fetch(url)).ok) return;
    } catch {}
    await sleep(300);
  }
  throw new Error(`dev server didn't come up at ${url}`);
}

before(async () => {
  if (!process.env.E2E_URL) {
    server = spawn("pnpm", ["exec", "vite", "--port", String(PORT), "--strictPort"], {
      cwd: new globalThis.URL("..", import.meta.url).pathname,
      stdio: "ignore",
      detached: true,
    });
  }
  await waitFor(URL);
  browser = await chromium.launch();
});

after(async () => {
  await browser?.close();
  if (server) process.kill(-server.pid);
});

async function fresh() {
  page = await browser.newPage({ viewport: { width: 1360, height: 860 } });
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(URL);
  await page.locator("text=Local Postgres").first().dblclick();
  await page.getByText("orders", { exact: true }).first().waitFor();
  return page;
}

const noErrors = () => assert.deepEqual(errors.splice(0), [], "no page errors");

describe("Rowhouse UI", () => {
  test("connects and opens a table", async () => {
    const p = await fresh();
    await p.getByText("users", { exact: true }).first().click();
    // the grid shows rows from the mock
    await p.locator("text=/@example\\.com/").first().waitFor({ timeout: 10_000 });
    noErrors();
    await p.close();
  });

  test("runs a query from a new tab", async () => {
    const p = await fresh();
    await p.keyboard.press("Control+t");
    const editor = p.locator(".cm-content").first();
    await editor.click();
    await p.keyboard.type("SELECT id, email FROM users");
    await p.keyboard.press("Control+Enter");
    await p.locator("text=/\\d+ rows?/").first().waitFor({ timeout: 10_000 });
    noErrors();
    await p.close();
  });

  test("dashboard: filters, snapshot to a destination", async () => {
    const p = await fresh();
    await p.keyboard.press("Control+k");
    await p.keyboard.type("Dashboard: Sales");
    await p.keyboard.press("Enter");
    await p.locator("text=Revenue · last 14 days").waitFor();
    // the filter bar collects :from / :region from the tiles
    await p.locator("text=:region").waitFor();
    await p.locator("button", { hasText: "Export" }).first().click();
    await p.locator("button", { hasText: "Schedule snapshots" }).click();
    await p.getByPlaceholder("https://discord.com/api/webhooks/…").or(p.getByPlaceholder("xoxb-… bot token")).first().waitFor();
    await p.locator("select").filter({ hasText: "Slack" }).last().selectOption("discord");
    await p.getByPlaceholder("https://discord.com/api/webhooks/…").fill("https://discord.com/api/webhooks/1/token");
    await p.getByRole("button", { name: "Add", exact: true }).click();
    await p.locator("text=discord.com/…oken").waitFor();
    const done = p.waitForEvent("console", { predicate: (m) => m.text().includes("snapshot done"), timeout: 20_000 });
    await p.locator("button", { hasText: "Save & send now" }).click();
    const msg = await done;
    assert.match(msg.text(), /error: null/);
    const bytes = await p.evaluate(() => String(window.__rhLastWrite ?? "").length);
    assert.ok(bytes > 10_000, `PNG rendered (${bytes} base64 chars)`);
    noErrors();
    await p.close();
  });

  test("connection form: new connectors and validation", async () => {
    const p = await fresh();
    await p.keyboard.press("Control+o");
    await p.locator("button", { hasText: "New connection" }).click();
    for (const k of ["Snowflake", "BigQuery", "Elasticsearch", "OpenSearch"]) {
      await p.locator("button", { hasText: k }).first().waitFor();
    }
    await p.locator("button", { hasText: "Snowflake" }).first().click();
    await p.getByPlaceholder("myorg-myaccount").waitFor();
    await p.getByPlaceholder("COMPUTE_WH").waitFor();
    await p.locator("button", { hasText: "BigQuery" }).first().click();
    await p.locator("text=Service-account key file").waitFor();
    noErrors();
    await p.close();
  });
});
