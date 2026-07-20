import * as RD from "@radix-ui/react-dialog";
import { AnimatePresence, motion } from "motion/react";
import { X } from "lucide-react";
import { cn } from "../../lib/utils";

export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  className,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <RD.Root open={open} onOpenChange={onOpenChange}>
      <AnimatePresence>
        {open && (
          <RD.Portal forceMount>
            <RD.Overlay asChild forceMount>
              <motion.div
                className="fixed inset-0 z-50 bg-black/50 backdrop-blur-sm"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.15 }}
              />
            </RD.Overlay>
            <RD.Content asChild forceMount>
              <motion.div
                className={cn(
                  "fixed left-1/2 top-1/2 z-50 w-[calc(100vw-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2",
                  "rounded-lg border border-border bg-card shadow-pop focus:outline-none",
                  "max-h-[calc(100vh-4rem)] overflow-hidden flex flex-col",
                  className,
                )}
                initial={{ opacity: 0, scale: 0.96, y: "-46%" }}
                animate={{ opacity: 1, scale: 1, y: "-50%" }}
                exit={{ opacity: 0, scale: 0.97, y: "-46%" }}
                transition={{ duration: 0.18, ease: [0.16, 1, 0.3, 1] }}
              >
                <div className="flex items-start justify-between gap-4 border-b border-border px-6 py-4">
                  <div className="min-w-0">
                    <RD.Title className="text-lg font-semibold">{title}</RD.Title>
                    {description && (
                      <RD.Description className="mt-0.5 text-sm text-muted-foreground">
                        {description}
                      </RD.Description>
                    )}
                  </div>
                  <RD.Close className="rounded-md p-1.5 text-muted-foreground transition hover:bg-muted hover:text-foreground">
                    <X className="h-4 w-4" />
                  </RD.Close>
                </div>
                <div className="overflow-y-auto px-6 py-5">{children}</div>
              </motion.div>
            </RD.Content>
          </RD.Portal>
        )}
      </AnimatePresence>
    </RD.Root>
  );
}
