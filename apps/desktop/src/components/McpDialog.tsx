import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type McpInfo } from "../lib/api";
import { Button, Modal } from "./ui";

export function McpDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [info, setInfo] = useState<McpInfo | null>(null);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (open) api.mcpInfo().then(setInfo);
  }, [open]);
  const snippet = info ? JSON.stringify(info.snippet, null, 2) : "";

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={520}
      title="Connect an AI assistant"
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">Only connections with “Expose via MCP” are visible.</span>
          <Button onClick={onClose}>Done</Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        <p className="text-muted">
          Rowhouse ships a read-only MCP server. Add it to Claude Desktop, Cursor, or any MCP client and the assistant can list
          tables, read schemas and run <span className="font-medium text-fg">SELECT</span> queries on your saved connections.
        </p>
        <div className="relative">
          <pre className="selectable overflow-x-auto rounded-lg border border-line bg-panel p-3 font-mono text-2xs leading-relaxed">{snippet}</pre>
          <Button
            size="xs"
            className="absolute top-2 right-2"
            onClick={() => {
              navigator.clipboard.writeText(snippet);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
          >
            {copied ? <Check size={11} /> : <Copy size={11} />} {copied ? "Copied" : "Copy"}
          </Button>
        </div>
        <ul className="flex flex-col gap-1 text-2xs text-muted">
          <li>• Tools: list_connections, list_tables, describe_table, sample_rows, run_query, execute_sql</li>
          <li>• Reads are read-only (keyword guard + read-only transaction)</li>
          <li>• execute_sql: writes only on connections with “Allow AI to request writes”, and each one waits for your approval here</li>
          <li className="truncate">
            • Profiles file: <span className="selectable font-mono">{info?.configPath}</span>
          </li>
        </ul>
      </div>
    </Modal>
  );
}
