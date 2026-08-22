import { forwardRef } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "../../lib/utils";

/** Card surface. */
export function Card({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("rounded-lg border border-border bg-card text-card-foreground", className)}
      {...props}
    />
  );
}

/** A small status/label pill. `tone` sets an accent color via inline style. */
export function Badge({
  className,
  tone,
  ...props
}: React.HTMLAttributes<HTMLSpanElement> & { tone?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium",
        className,
      )}
      style={
        tone
          ? { color: tone, background: `color-mix(in oklch, ${tone} 14%, transparent)` }
          : undefined
      }
      {...props}
    />
  );
}

/** A colored dot, e.g. next to a status label. */
export function Dot({ tone, className }: { tone: string; className?: string }) {
  return (
    <span
      className={cn("inline-block h-2 w-2 shrink-0 rounded-full", className)}
      style={{ background: tone }}
    />
  );
}

export const Input = forwardRef<HTMLInputElement, React.InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        "h-10 w-full rounded-full border border-input bg-background px-4 text-sm",
        "placeholder:text-muted-foreground/60 transition-all duration-200",
        "focus-visible:outline-none focus-visible:border-primary",
        "focus-visible:shadow-[0_0_0_4px_var(--ring-soft,rgba(124,92,255,0.15))]",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  React.TextareaHTMLAttributes<HTMLTextAreaElement>
>(({ className, ...props }, ref) => (
  <textarea
    ref={ref}
    className={cn(
      "w-full rounded-2xl border border-input bg-background px-4 py-2.5 text-sm transition-all duration-200",
      "focus-visible:outline-none focus-visible:border-primary",
      "focus-visible:shadow-[0_0_0_4px_var(--ring-soft,rgba(124,92,255,0.15))]",
      "placeholder:text-muted-foreground/60 resize-y min-h-20",
      "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      className,
    )}
    {...props}
  />
));
Textarea.displayName = "Textarea";

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{label}</span>
      {children}
      {hint && <span className="text-xs text-muted-foreground">{hint}</span>}
    </label>
  );
}

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={cn("animate-spin", className)} />;
}
