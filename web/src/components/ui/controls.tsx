import { Check } from "lucide-react";
import { cn } from "../../lib/utils";

/** A toggleable chip — the click-first primitive for multi-select traits. */
export function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-3 py-1.5 text-sm transition cursor-pointer",
        active
          ? "border-primary bg-primary/10 text-primary font-medium"
          : "border-border text-muted-foreground hover:border-muted-foreground/40 hover:text-foreground",
      )}
    >
      {active && <Check className="h-3.5 w-3.5" />}
      {children}
    </button>
  );
}

/** A segmented single-select — click-first alternative to a dropdown. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
}: {
  options: { value: T; label: string; dot?: boolean }[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <div className="inline-flex rounded-full border border-border bg-muted/50 p-0.5">
      {options.map((opt) => (
        <button
          key={opt.value}
          type="button"
          onClick={() => onChange(opt.value)}
          className={cn(
            "relative rounded-full px-3 py-1.5 text-sm transition cursor-pointer",
            value === opt.value
              ? // The chosen one wears the accent (owner's word): you always
                // know which room you are in.
                "bg-primary text-primary-foreground font-medium shadow-[0_4px_14px_-6px_var(--ring-soft,rgba(124,92,255,0.7))]"
              : "text-muted-foreground hover:text-foreground",
          )}
        >
          {opt.label}
          {opt.dot && value !== opt.value && (
            // Something unseen lives in this room (ADR-0041: an agent may
            // now write unprompted, and a word nobody sees helps nobody).
            <span className="absolute -right-0 -top-0 h-2 w-2 rounded-full bg-primary shadow-[0_0_0_2px_var(--color-background)]" />
          )}
        </button>
      ))}
    </div>
  );
}
