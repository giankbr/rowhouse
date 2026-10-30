import { isTauri } from "./api";

export interface UpdateInfo {
  version: string;
  notes?: string;
  install: (onProgress?: (pct: number) => void) => Promise<void>;
}

/**
 * Ask the update server (GitHub Releases `latest.json`) for a newer build.
 * Resolves to null when there's none, or when auto-update isn't configured
 * for this build (no signing key yet → the updater plugin isn't loaded).
 */
export async function checkForUpdate(): Promise<UpdateInfo | null> {
  if (!isTauri) return null;
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const update = await check();
    if (!update) return null;
    return {
      version: update.version,
      notes: update.body ?? undefined,
      install: async (onProgress) => {
        let total = 0;
        let done = 0;
        await update.downloadAndInstall((e) => {
          if (e.event === "Started") total = e.data.contentLength ?? 0;
          if (e.event === "Progress") {
            done += e.data.chunkLength;
            if (total) onProgress?.(Math.round((done / total) * 100));
          }
        });
        const { relaunch } = await import("@tauri-apps/plugin-process");
        await relaunch();
      },
    };
  } catch {
    return null;
  }
}
