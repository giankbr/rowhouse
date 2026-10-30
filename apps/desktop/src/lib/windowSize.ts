/**
 * The start screen is a small window (like TablePlus' welcome window); opening a
 * connection grows it to the workspace size the user last had.
 */
import { isTauri } from "./api";

const KEY = "rh-workspace-size";
const LAUNCHER = { w: 860, h: 540, minW: 760, minH: 480 };
const WORKSPACE = { w: 1280, h: 800, minW: 900, minH: 560 };

let mode: "launcher" | "workspace" = "launcher";

export async function setWindowMode(next: "launcher" | "workspace") {
  if (!isTauri || next === mode) return;
  const { getCurrentWindow, LogicalSize } = await import("@tauri-apps/api/window");
  const win = getCurrentWindow();
  try {
    if (next === "workspace") {
      let size = { w: WORKSPACE.w, h: WORKSPACE.h };
      try {
        size = { ...size, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
      } catch {
        /* default size */
      }
      await win.setMinSize(new LogicalSize(WORKSPACE.minW, WORKSPACE.minH));
      await win.setSize(new LogicalSize(size.w, size.h));
    } else {
      // remember the workspace size for next time
      const scale = await win.scaleFactor();
      const cur = (await win.innerSize()).toLogical(scale);
      if (cur.width >= WORKSPACE.minW) {
        try {
          localStorage.setItem(KEY, JSON.stringify({ w: Math.round(cur.width), h: Math.round(cur.height) }));
        } catch {
          /* not saved */
        }
      }
      await win.setMinSize(new LogicalSize(LAUNCHER.minW, LAUNCHER.minH));
      await win.setSize(new LogicalSize(LAUNCHER.w, LAUNCHER.h));
    }
    await win.center();
    mode = next;
  } catch (e) {
    console.warn("window resize failed", e);
  }
}
