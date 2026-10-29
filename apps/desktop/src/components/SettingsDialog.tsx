import { CheckCircle2, Loader2, ShieldCheck, XCircle } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type AiSettings } from "../lib/api";
import { Button, Field, Input, Modal, Segmented, Switch, cx } from "./ui";

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
      title="Settings · AI"
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
