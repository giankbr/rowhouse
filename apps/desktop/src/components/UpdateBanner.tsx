import { Download, Loader2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { checkForUpdate, type UpdateInfo } from "../lib/updates";
import { Button } from "./ui";

/** Checks once at start-up (and every 6 hours); shows a small banner when a new version is out. */
export function UpdateBanner() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const run = () => checkForUpdate().then(setUpdate);
    const first = setTimeout(run, 4000);
    const every = setInterval(run, 6 * 60 * 60 * 1000);
    return () => (clearTimeout(first), clearInterval(every));
  }, []);

  if (!update || dismissed) return null;
  return (
    <div className="animate-pop fixed right-4 bottom-9 z-50 flex w-80 flex-col gap-2 rounded-xl bg-elevated p-3 text-xs shadow-pop">
      <div className="flex items-start gap-2">
        <Download size={14} className="mt-0.5 shrink-0 text-accent" />
        <div className="flex-1">
          <div className="font-medium">Rowhouse {update.version} is available</div>
          {update.notes && <div className="mt-0.5 line-clamp-3 text-muted">{update.notes}</div>}
          {error && <div className="mt-1 text-danger">{error}</div>}
        </div>
        <button onClick={() => setDismissed(true)} className="text-subtle hover:text-fg" title="Later">
          <X size={13} />
        </button>
      </div>
      <Button
        variant="primary"
        size="xs"
        disabled={progress !== null}
        onClick={async () => {
          setProgress(0);
          try {
            await update.install(setProgress);
          } catch (e) {
            setError(String(e));
            setProgress(null);
          }
        }}
      >
        {progress !== null ? (
          <>
            <Loader2 size={11} className="animate-spin" /> Downloading {progress}%
          </>
        ) : (
          "Install & restart"
        )}
      </Button>
    </div>
  );
}
