import { useEffect } from "react";
import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";
import type { Notification } from "../lib/api";
import { useNotificationText, useT } from "../lib/i18n";

/**
 * Live notifications, surfaced the moment they arrive over `/ws` (ADR-0020).
 * Purely a fast path: every toast is already a durable row in the inbox, so
 * dismissing one loses nothing.
 */
export function Toaster({
  toasts,
  onDismiss,
  onOpen,
}: {
  toasts: Notification[];
  onDismiss: (id: string) => void;
  onOpen: () => void;
}) {
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-80 flex-col gap-2">
      <AnimatePresence initial={false}>
        {toasts.map((t) => (
          <Toast key={t.id} toast={t} onDismiss={onDismiss} onOpen={onOpen} />
        ))}
      </AnimatePresence>
    </div>
  );
}

function Toast({
  toast,
  onDismiss,
  onOpen,
}: {
  toast: Notification;
  onDismiss: (id: string) => void;
  onOpen: () => void;
}) {
  const t = useT();
  const text = useNotificationText()(toast);
  // Anything that needs a decision stays until you deal with it.
  const sticky = !!toast.approval_id;
  useEffect(() => {
    if (sticky) return;
    const t = setTimeout(() => onDismiss(toast.id), 8000);
    return () => clearTimeout(t);
  }, [toast.id, sticky, onDismiss]);

  return (
    <motion.div
      layout
      initial={{ opacity: 0, x: 24, scale: 0.97 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, x: 24, scale: 0.97 }}
      transition={{ duration: 0.2, ease: [0.16, 1, 0.3, 1] }}
      className="pointer-events-auto overflow-hidden rounded-lg border border-border bg-card shadow-pop"
    >
      <button onClick={onOpen} className="block w-full px-4 py-3 text-left">
        <div className="flex items-start gap-2">
          <p className="min-w-0 flex-1 text-sm font-medium">{text.title}</p>
          <span
            role="button"
            tabIndex={0}
            aria-label={t("common.dismiss")}
            onClick={(e) => {
              e.stopPropagation();
              onDismiss(toast.id);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.stopPropagation();
                onDismiss(toast.id);
              }
            }}
            className="-mr-1 -mt-0.5 shrink-0 rounded p-1 text-muted-foreground transition hover:bg-muted hover:text-foreground"
          >
            <X className="h-3.5 w-3.5" />
          </span>
        </div>
        <p className="mt-1 line-clamp-3 whitespace-pre-wrap text-xs leading-relaxed text-muted-foreground">
          {text.body}
        </p>
        {sticky && (
          <p className="mt-2 text-xs font-medium text-primary">{t("inbox.toastWaiting")}</p>
        )}
      </button>
    </motion.div>
  );
}
