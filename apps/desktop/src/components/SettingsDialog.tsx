import { CheckCircle2, Loader2, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type AiSettings } from "../lib/api";
import { Button, Field, Input, Modal, Segmented, Switch, cx } from "./ui";
import type { LicenseStatus } from "../lib/api";
import { saveAppearance, useAppearance, type Appearance, type Density, type MonoFont, type UiFont } from "../lib/appearance";

const PRESETS: { id: string; label: string; provider: AiSettings["provider"]; baseUrl: string; needsKey: boolean }[] = [
  { id: "openai", label: "OpenAI", provider: "openai", baseUrl: "https://api.openai.com/v1", needsKey: true },
  { id: "anthropic", label: "Anthropic", provider: "anthropic", baseUrl: "https://api.anthropic.com/v1", needsKey: true },
  { id: "openrouter", label: "OpenRouter", provider: "openai", baseUrl: "https://openrouter.ai/api/v1", needsKey: true },
  { id: "ollama", label: "Ollama (local)", provider: "openai", baseUrl: "http://localhost:11434/v1", needsKey: false },
  { id: "lmstudio", label: "LM Studio", provider: "openai", baseUrl: "http://localhost:1234/v1", needsKey: false },
];

export function SettingsDialog({ open, onClose, onSaved }: { open: boolean; onClose: () => void; onSaved: (s: AiSettings) => void }) {
  const [s, setS] = useState<AiSettings | null>(null);
  const [hasKey, setHasKey] = useState(false);
  const [key, setKey] = useState("");
  const [removeKey, setRemoveKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [status, setStatus] = useState<{ state: "idle" | "busy" | "ok" | "err"; msg?: string }>({ state: "idle" });

  useEffect(() => {
    if (!open) return;
    setKey("");
    setRemoveKey(false);
    setStatus({ state: "idle" });
    api.ai.getSettings().then((r) => (setS(r.settings), setHasKey(r.hasKey)));
  }, [open]);

  if (!s) return null;
  const set = (p: Partial<AiSettings>) => setS({ ...s, ...p });
  // undefined = keep the stored key, "" = remove it
  const keyArg = removeKey ? "" : key === "" ? undefined : key;
  const preset = PRESETS.find((p) => p.baseUrl === s.baseUrl.replace(/\/$/, "") && p.provider === s.provider);

  const loadModels = async () => {
    setStatus({ state: "busy", msg: "Loading models…" });
    try {
      const m = await api.ai.listModels(s, keyArg);
      setModels(m);
      setStatus({ state: "ok", msg: `${m.length} models available` });
    } catch (e) {
      setStatus({ state: "err", msg: String(e) });
    }
  };
  const test = async () => {
    setStatus({ state: "busy", msg: "Testing…" });
    try {
      const r = await api.ai.test(s, keyArg);
      setStatus({ state: "ok", msg: `Model replied: ${r.slice(0, 40)}` });
    } catch (e) {
      setStatus({ state: "err", msg: String(e) });
    }
  };
  const save = async () => {
    await api.ai.saveSettings(s, keyArg);
    onSaved(s);
    onClose();
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={520}
      title="Settings"
      footer={
        <>
          {status.state !== "idle" && (
            <span
              title={status.msg}
              className={cx(
                "flex max-w-64 items-center gap-1 truncate text-2xs",
                status.state === "ok" && "text-success",
                status.state === "err" && "text-danger",
                status.state === "busy" && "text-muted",
              )}
            >
              {status.state === "busy" && <Loader2 size={12} className="animate-spin" />}
              {status.state === "ok" && <CheckCircle2 size={12} />}
              {status.state === "err" && <XCircle size={12} />}
              <span className="truncate">{status.msg}</span>
            </span>
          )}
          <span className="flex-1" />
          <Button onClick={test} disabled={!s.model || status.state === "busy"}>
            Test
          </Button>
          <Button variant="primary" onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <AppearanceSection />
        <div className="h-px bg-line" />
        <LicenseSection />
        <div className="h-px bg-line" />
        <h3 className="text-2xs font-medium tracking-wide text-subtle uppercase">AI</h3>
        <Switch checked={s.enabled} onChange={(v) => set({ enabled: v })} label="Enable AI features (Ask AI, Fix with AI, Explain)" />
        <div className={cx("flex flex-col gap-3 transition-opacity", !s.enabled && "pointer-events-none opacity-45")}>
          <div className="flex flex-wrap gap-1">
            {PRESETS.map((p) => (
              <button
                key={p.id}
                type="button"
                onClick={() => (set({ provider: p.provider, baseUrl: p.baseUrl }), setModels([]))}
                className={cx(
                  "h-6 rounded-md px-2 text-2xs font-medium",
                  preset?.id === p.id ? "bg-accent-soft text-fg ring-1 ring-accent" : "bg-panel text-muted ring-1 ring-line hover:text-fg",
                )}
              >
                {p.label}
              </button>
            ))}
          </div>
          <div className="flex items-end gap-2">
            <Field label="API style">
              <Segmented
                value={s.provider}
                onChange={(provider) => set({ provider })}
                options={[
                  { value: "openai", label: "OpenAI-compatible" },
                  { value: "anthropic", label: "Anthropic" },
                ]}
              />
            </Field>
            <div className="flex-1">
              <Field label="Base URL">
                <Input value={s.baseUrl} onChange={(e) => set({ baseUrl: e.target.value })} />
              </Field>
            </div>
          </div>
          <Field
            label="API key"
            hint={
              removeKey
                ? "The stored key will be removed when you save."
                : hasKey
                  ? "Stored in the system keychain. Leave empty to keep it, or type a new one."
                  : "Stored in the system keychain. Not needed for local models."
            }
          >
            <div className="flex gap-2">
              <Input
                type="password"
                value={key}
                disabled={removeKey}
                placeholder={hasKey && !removeKey ? "•••••••• (stored)" : preset?.needsKey === false ? "not required" : "sk-…"}
                onChange={(e) => setKey(e.target.value)}
              />
              {hasKey && (
                <Button variant={removeKey ? "default" : "ghost"} onClick={() => (setRemoveKey((r) => !r), setKey(""))}>
                  {removeKey ? "Keep key" : "Remove"}
                </Button>
              )}
            </div>
          </Field>
          <Field label="Model">
            <div className="flex gap-2">
              <Input list="rh-models" value={s.model} placeholder="model id" onChange={(e) => set({ model: e.target.value })} />
              <datalist id="rh-models">
                {models.map((m) => (
                  <option key={m} value={m} />
                ))}
              </datalist>
              <Button onClick={loadModels} disabled={status.state === "busy"}>
                Load list
              </Button>
            </div>
          </Field>
          <Switch
            checked={!!s.inlineCompletions}
            onChange={(v) => set({ inlineCompletions: v })}
            label={
              <span className="text-xs">
                Inline suggestions while typing <span className="text-subtle">— Tab to accept; one small request per pause</span>
              </span>
            }
          />
          <div className="flex gap-2 rounded-lg bg-panel p-3 text-2xs text-muted">
            <ShieldCheck size={14} className="mt-px shrink-0 text-success" />
            <span>
              Only your schema (table and column names and types) plus the SQL / error you're working on is sent to the model.
              Row data never leaves your machine, and nothing the AI suggests runs until you accept it.
            </span>
          </div>
        </div>
      </div>
    </Modal>
  );
}

function AppearanceSection() {
  const a = useAppearance();
  const set = (patch: Partial<Appearance>) => saveAppearance({ ...a, ...patch });
  const row = "flex items-center gap-3";
  const label = "w-24 shrink-0 text-xs text-muted";
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-2xs font-medium tracking-wide text-subtle uppercase">Appearance</h3>
      <div className={row}>
        <span className={label}>Interface font</span>
        <Segmented<UiFont>
          value={a.uiFont}
          onChange={(uiFont) => set({ uiFont })}
          options={[
            { value: "inter", label: <span style={{ fontFamily: "Inter Variable" }}>Inter</span> },
            { value: "geist", label: <span style={{ fontFamily: "Geist Variable" }}>Geist</span> },
            { value: "system", label: <span style={{ fontFamily: "-apple-system, system-ui" }}>System</span> },
          ]}
        />
      </div>
      <div className={row}>
        <span className={label}>Code font</span>
        <Segmented<MonoFont>
          value={a.monoFont}
          onChange={(monoFont) => set({ monoFont })}
          options={[
            { value: "jetbrains", label: <span style={{ fontFamily: "JetBrains Mono Variable" }}>JetBrains Mono</span> },
            { value: "geist", label: <span style={{ fontFamily: "Geist Mono Variable" }}>Geist Mono</span> },
            { value: "system", label: <span style={{ fontFamily: "ui-monospace, SF Mono, Menlo" }}>SF Mono</span> },
          ]}
        />
      </div>
      <div className={row}>
        <span className={label}>Query timeout</span>
        <Segmented<string>
          value={String(a.queryTimeoutSec)}
          onChange={(v) => set({ queryTimeoutSec: Number(v) })}
          options={[
            { value: "0", label: "Off" },
            { value: "30", label: "30 s" },
            { value: "120", label: "2 min" },
            { value: "600", label: "10 min" },
          ]}
        />
      </div>
      <div className={row}>
        <span className={label}>Density</span>
        <Segmented<Density>
          value={a.density}
          onChange={(density) => set({ density })}
          options={[
            { value: "compact", label: "Compact" },
            { value: "comfortable", label: "Comfortable" },
          ]}
        />
      </div>
    </div>
  );
}

function LicenseSection() {
  const [st, setSt] = useState<LicenseStatus | null>(null);
  const [key, setKey] = useState("");
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api.license.status().then(setSt).catch(() => {});
  }, []);
  if (!st) return null;
  const date = (t: number) => new Date(t * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
  return (
    <div className="flex flex-col gap-2">
      <h3 className="text-2xs font-medium tracking-wide text-subtle uppercase">License</h3>
      {st.communityBuild ? (
        <p className="text-xs text-muted">Community build — every feature is unlocked. Official releases ask for a Pro license for scheduled queries and AI write requests.</p>
      ) : st.license ? (
        <div className="flex items-center gap-3 rounded-lg border border-line bg-panel px-3 py-2 text-xs">
          <span className="rounded bg-accent px-1.5 py-px text-[10px] font-semibold text-accent-fg uppercase">{st.license.plan}</span>
          <span className="min-w-0 flex-1 truncate">
            {st.license.email}
            <span className="text-muted">
              {" "}
              · updates until {date(st.license.updatesUntil)}
              {st.updatesExpired && " (expired — this build still works)"}
            </span>
          </span>
          <Button size="xs" variant="ghost" onClick={() => api.license.deactivate().then(setSt)}>
            Remove
          </Button>
        </div>
      ) : (
        <form
          className="flex flex-col gap-1.5"
          onSubmit={async (e) => {
            e.preventDefault();
            setErr(null);
            try {
              setSt(await api.license.activate(key));
              setKey("");
            } catch (x) {
              setErr(String(x));
            }
          }}
        >
          <p className="text-xs text-muted">Pro unlocks scheduled queries and AI write requests over MCP. Paste the key from your purchase email.</p>
          <div className="flex gap-2">
            <Input value={key} onChange={(e) => setKey(e.target.value)} placeholder="RH1-…" className="font-mono" />
            <Button type="submit" variant="primary" disabled={!key.trim()}>
              Activate
            </Button>
          </div>
          {(err || st.error) && <span className="text-2xs text-danger">{err ?? st.error}</span>}
        </form>
      )}
    </div>
  );
}
