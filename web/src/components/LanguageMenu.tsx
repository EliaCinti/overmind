import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Check, Languages } from "lucide-react";
import type { LanguageCode } from "../lib/api";
import { useT } from "../lib/i18n";
import { cn } from "../lib/utils";

/**
 * Languages are named in their own language — a speaker finds "Italiano"
 * faster than "Italian", and a list of endonyms needs no translation.
 *
 * Deliberately no flags: a flag is a country, and languages are not countries.
 * English is not the United Kingdom, and choosing one flag for it would be
 * picking a side in someone's argument.
 */
const LANGUAGES: { code: LanguageCode; name: string }[] = [
  { code: "en", name: "English" },
  { code: "it", name: "Italiano" },
];

export function LanguageMenu({
  language,
  onChange,
}: {
  language: LanguageCode;
  onChange: (code: LanguageCode) => void;
}) {
  const t = useT();
  const current = LANGUAGES.find((l) => l.code === language) ?? LANGUAGES[0];

  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <button
          className="inline-flex h-9 w-9 items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={`${t("nav.language")}: ${current.name}`}
          title={`${t("nav.language")}: ${current.name}`}
        >
          {/* Icon only: the whole interface already tells you which language is
              active, and a rarely-used control should not crowd the bar. The
              current one is marked in the menu. */}
          <Languages className="h-4.5 w-4.5" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          sideOffset={6}
          className="z-50 min-w-44 rounded-lg border border-border bg-card p-1 shadow-pop"
        >
          <DropdownMenu.Label className="px-2 py-1.5 text-xs text-muted-foreground">
            {t("nav.language")}
          </DropdownMenu.Label>
          {LANGUAGES.map((l) => {
            const active = l.code === language;
            return (
              <DropdownMenu.Item
                key={l.code}
                onSelect={() => !active && onChange(l.code)}
                className={cn(
                  "flex cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none",
                  "data-[highlighted]:bg-muted",
                  active && "font-medium",
                )}
              >
                <Check className={cn("h-4 w-4", active ? "text-primary" : "opacity-0")} />
                <span lang={l.code}>{l.name}</span>
              </DropdownMenu.Item>
            );
          })}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
