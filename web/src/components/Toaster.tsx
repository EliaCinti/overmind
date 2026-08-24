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
  const t = useT();
  // The stack is bottom-anchored, so it grows *upward* — and approval toasts
  // are sticky, so several of them used to grow past the top of the screen:
  // the last one always visible, the first unreachable (measured). At most
  // three are shown; the rest compact into one summary chip that opens the
  // inbox, and the container can never exceed the viewport.
  const visible = toasts.slice(-3);
  const hidden = toasts.length - visible.length;
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex max-h-[calc(100vh-2rem)] w-80 flex-col justify-end gap-2 overflow-hidden">
      <AnimatePresence initial={false}>
        {hidden > 0 && (
          <motion.button
            key="more"
            layout
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onOpen}
            className="pointer-events-auto self-end rounded-full border border-border bg-card px-3 py-1.5 text-xs text-muted-foreground shadow-pop transition hover:text-foreground"
          >
            {t("inbox.toastMore", { n: hidden })}
          </motion.button>
        )}
        {visible.map((x) => (
          <Toast key={x.id} toast={x} onDismiss={onDismiss} onOpen={onOpen} />
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
