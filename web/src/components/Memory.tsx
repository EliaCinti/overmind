import { useEffect, useState } from "react";
import { BrainCircuit, Gavel, ListChecks, Search, Users } from "lucide-react";
import type { MemoryItem, MemoryPage } from "../lib/api";
import { api } from "../lib/api";
import { Badge } from "./ui/primitives";
import { Segmented } from "./ui/controls";
import { useFormats, useT } from "../lib/i18n";

type Kind = "memories" | "decisions";

/**
 * What the organization knows (M8, ADR-0025) — the company's own brain, read
 * back. Each row carries the task or meeting that produced it, which is the
 * difference between a pile of notes and a record you can act on.
 *
 * The four ways this page can be empty are four different problems, so it never
 * renders a bare "nothing here": see `Empty` below.
 */
export function Memory({ companyId, tick }: { companyId: string; tick: number }) {
  const t = useT();
  const [kind, setKind] = useState<Kind>("memories");
  const [typed, setTyped] = useState("");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState<MemoryPage | null>(null);
  const [loading, setLoading] = useState(true);

  // Debounced, because every keystroke would otherwise be a semantic search
  // against a live provider — the one call here that is not cheap.
  useEffect(() => {
    const id = setTimeout(() => setQuery(typed.trim()), 300);
    return () => clearTimeout(id);
  }, [typed]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    api
      .browseMemory(companyId, kind, query || undefined)
      .then((p) => alive && setPage(p))
      .catch(() => alive && setPage(null))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [companyId, kind, query, tick]);

  const items = page?.items ?? [];

  return (
    // Capped rather than full-bleed: these rows are prose, and a memory that
    // runs the width of a 27-inch display is one nobody reads.
    <div className="mx-auto flex h-full w-full max-w-4xl flex-col gap-4 overflow-hidden px-6 pb-6">
      <div className="flex flex-wrap items-center gap-3">
        <Segmented<Kind>
          value={kind}
          onChange={setKind}
          options={[
            { value: "memories", label: t("memory.memories") },
            { value: "decisions", label: t("memory.decisions") },
          ]}
        />
        <div className="relative min-w-0 flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground/60" />
          <input
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            placeholder={t("memory.search")}
            className="h-9 w-full rounded-md border border-input bg-background pl-9 pr-3 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          />
        </div>
      </div>

      {page && items.length === 0 && !loading ? (
        <Empty state={page.state} kind={kind} searching={!!query} />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto pr-1">
          {items.map((item, i) => (
            <Row key={item.id ?? `row-${i}`} item={item} />
          ))}
        </div>
      )}
    </div>
  );
}

function Row({ item }: { item: MemoryItem }) {
  const t = useT();
  const { timeAgo } = useFormats();
  const subject = item.subject;
  return (
    <article className="rounded-lg border border-border bg-card p-4">
      <div className="flex items-start justify-between gap-3">
        <h3 className="min-w-0 flex-1 text-sm font-medium">{item.title}</h3>
        {item.category && (
          <Badge tone="var(--color-primary)">{item.category}</Badge>
        )}
      </div>

      {item.content && (
        <p className="mt-2 line-clamp-3 whitespace-pre-wrap text-sm text-muted-foreground">
          {item.content}
        </p>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {subject ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-muted/60 px-2 py-1">
            {subject.type === "task" ? (
              <ListChecks className="h-3.5 w-3.5" />
            ) : (
              <Users className="h-3.5 w-3.5" />
            )}
            <span className="opacity-70">
              {subject.type === "task" ? t("memory.fromTask") : t("memory.fromMeeting")}
            </span>
            <span className="max-w-60 truncate font-medium text-foreground">{subject.title}</span>
          </span>
        ) : (
          // Said plainly rather than left blank: a memory with no recorded
          // source is a real state, not a rendering gap (ADR-0025).
          <span className="italic opacity-60">{t("memory.noSubject")}</span>
        )}
        {item.created_at && <span>· {timeAgo(item.created_at)}</span>}
      </div>
    </article>
  );
}

/** An empty page that says which kind of empty it is. */
function Empty({
  state,
  kind,
  searching,
}: {
  state: MemoryPage["state"];
  kind: Kind;
  searching: boolean;
}) {
  const t = useT();
  const { icon: Icon, title, body } = (() => {
    switch (state) {
      case "no_provider":
        return {
          icon: BrainCircuit,
          title: t("memory.noProviderTitle"),
          body: t("memory.noProviderBody"),
        };
      case "brain_off":
        return {
          icon: BrainCircuit,
          title: t("memory.brainOffTitle"),
          body: t("memory.brainOffBody"),
        };
      case "not_browsable":
        return {
          icon: Gavel,
          title: t("memory.notBrowsableTitle"),
          body: t("memory.notBrowsableBody"),
        };
      default:
        if (searching) {
          return {
            icon: Search,
            title: t("memory.noResultsTitle"),
            body: t("memory.noResultsBody"),
          };
        }
        // "Nothing remembered yet" would be false on the decisions tab of a
        // company that has memories and simply has not held a meeting.
        return kind === "decisions"
          ? {
              icon: Gavel,
              title: t("memory.emptyDecisionsTitle"),
              body: t("memory.emptyDecisionsBody"),
            }
          : {
              icon: BrainCircuit,
              title: t("memory.emptyTitle"),
              body: t("memory.emptyBody"),
            };
    }
  })();

  return (
    <div className="flex flex-1 items-center justify-center">
      <div className="max-w-sm text-center">
        <Icon className="mx-auto h-8 w-8 text-muted-foreground/40" />
        <p className="mt-3 text-sm font-medium">{title}</p>
        <p className="mt-1 text-sm text-muted-foreground">{body}</p>
      </div>
    </div>
  );
}
