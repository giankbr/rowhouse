import { useCallback, useEffect, useState } from "react";
import { api, type ConnectionConfig, type Dashboard } from "../lib/api";
import { DashboardView } from "./DashboardView";

const pad = (n: number) => String(n).padStart(2, "0");
const stamp = (d = new Date()) => `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`;
const TIMEOUT_MS = 120_000;

async function blobToBase64(blob: Blob) {
  const buf = new Uint8Array(await blob.arrayBuffer());
  let bin = "";
  for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
  return btoa(bin);
}

/**
 * Scheduled dashboard snapshots. The backend says when one is due; we render
 * the dashboard offscreen, turn it into a PNG and write it to the folder.
 * One at a time, so a burst of due dashboards doesn't hammer the database.
 */
export function SnapshotRunner({ connections }: { connections: ConnectionConfig[] }) {
  const [queue, setQueue] = useState<Dashboard[]>([]);
  const current = queue[0];

  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    api.dashboards
      .onSnapshotDue((d) => setQueue((q) => (q.some((x) => x.id === d.id) ? q : [...q, d])))
      .then((f) => (dead ? f() : (off = f)));
    return () => {
      dead = true;
      off?.();
    };
  }, []);

  const finish = useCallback(async (d: Dashboard, path: string | null, error: string | null) => {
    await api.dashboards.snapshotDone(d.id, path, error).catch(() => {});
    setQueue((q) => q.filter((x) => x.id !== d.id));
  }, []);

  // never wedge the queue on a dashboard that won't finish loading
  useEffect(() => {
    if (!current) return;
    const t = setTimeout(() => finish(current, null, "timed out waiting for tiles"), TIMEOUT_MS);
    return () => clearTimeout(t);
  }, [current, finish]);

  const onReady = useCallback(
    async (node: HTMLElement, dash: Dashboard) => {
      const snap = dash.snapshot;
      if (!snap) return finish(dash, null, "snapshot settings missing");
      try {
        const { toBlob } = await import("html-to-image");
        const blob = await toBlob(node, { pixelRatio: 2, backgroundColor: getComputedStyle(document.body).backgroundColor });
        if (!blob) throw "couldn't render the dashboard";
        const dir = snap.dir.replace(/[\\/]+$/, "");
        const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
        const path = `${dir}${sep}${dash.name.replace(/[^\w.-]+/g, "-")}-${stamp()}.png`;
        await api.writeBase64File(path, await blobToBase64(blob));
        await finish(dash, path, null);
      } catch (e) {
        await finish(dash, null, String(e));
      }
    },
    [finish],
  );

  if (!current) return null;
  return (
    <div aria-hidden className="pointer-events-none fixed top-0 flex h-[2000px] w-[1280px] flex-col bg-bg" style={{ left: -20000 }}>
      <DashboardView
        key={current.id}
        headless
        dashboardId={current.id}
        conn={connections.find((c) => c.id === current.connectionId)}
        onOpenQuery={() => {}}
        onToast={() => {}}
        onDeleted={() => {}}
        onSnapshotReady={onReady}
      />
    </div>
  );
}
