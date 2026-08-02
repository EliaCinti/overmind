import { useState } from "react";
import { ChevronRight, Download } from "lucide-react";
import type { Artifact } from "../lib/api";
import { api } from "../lib/api";
import { Spinner } from "./ui/primitives";
import { useFormats, useT } from "../lib/i18n";
import { formatBytes, iconFor, splitPath } from "../lib/files";
import { cn } from "../lib/utils";

/**
 * What a run handed back (M17).
 *
 * A deliverable is not always prose. An agent can produce a document, a
 * dataset, a chart, a source file — and the panel's job is to show each one as
 * what it is: prose and code read in place, an image is looked at, and a
 * spreadsheet is taken away. The old panel could only do the first, and showed
 * a filesystem path for everything else, which is a dead end dressed as
 * information.
 *
 * Everything is downloadable regardless of how it renders, because the file is
 * the deliverable and the preview is a convenience.
 */
export function Deliverables({ artifacts }: { artifacts: Artifact[] | null }) {
  const t = useT();
  if (artifacts === null) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner className="h-4 w-4" /> {t("task.loadingDocs")}
      </div>
    );
  }
  if (artifacts.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("task.noDocs")}</p>;
  }
  return (
    <div className="flex flex-col gap-2.5">
      {artifacts.map((a) => (
        <ArtifactRow key={a.id} artifact={a} />
      ))}
    </div>
  );
}

/** One deliverable: always a header, expandable when there is something to see. */
function ArtifactRow({ artifact }: { artifact: Artifact }) {
  const t = useT();
  const { locale } = useFormats();
  const isImage = artifact.mime.startsWith("image/");
  const hasText = artifact.content !== null && artifact.content.length > 0;
  const previewable = isImage || hasText;
  // Prose and code open by default — the reason you looked. A picture is small
  // enough to always show. Anything else is a row you act on, not read.
  const [open, setOpen] = useState(previewable);

  const { dir, name } = splitPath(artifact.title);
  const Icon = iconFor(artifact.mime);

  return (
    <div className="overflow-hidden rounded-md border border-border bg-muted/40">
      <div className="flex items-center gap-2 border-b border-border px-3 py-2">
        {previewable ? (
          <button
            type="button"
            onClick={() => setOpen((v) => !v)}
            className="-ml-1 rounded p-1 text-muted-foreground transition hover:bg-muted hover:text-foreground"
            aria-label={open ? t("task.collapse") : t("task.expand")}
            aria-expanded={open}
          >
            <ChevronRight className={cn("h-4 w-4 transition-transform", open && "rotate-90")} />
          </button>
        ) : (
          <span className="w-6" />
        )}
        <Icon className="h-4 w-4 shrink-0 text-primary" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {/* The folder the agent chose is part of the answer, but the file is
              what you are looking for — so the path is present and quiet. */}
          {dir && <span className="font-normal text-muted-foreground">{dir}/</span>}
          {name}
        </span>
        <span className="mono shrink-0 text-[11px] text-muted-foreground">
          {formatBytes(artifact.size_bytes, locale)}
        </span>
        {artifact.downloadable && (
          <a
            href={api.artifactUrl(artifact.id)}
            download={name}
            className="shrink-0 rounded p-1 text-muted-foreground transition hover:bg-muted hover:text-foreground"
            title={t("task.download")}
            aria-label={t("task.downloadNamed", { name })}
          >
            <Download className="h-4 w-4" />
          </a>
        )}
      </div>

      {open && isImage && (
        <a href={api.artifactUrl(artifact.id)} target="_blank" rel="noopener" className="block">
          <img
            src={api.artifactUrl(artifact.id)}
            alt={artifact.title}
            className="max-h-96 w-full bg-background object-contain p-2"
          />
        </a>
      )}
      {open && !isImage && hasText && (
        <pre className="mono max-h-96 overflow-auto whitespace-pre-wrap p-3 text-xs leading-relaxed">
          {artifact.content}
        </pre>
      )}
    </div>
  );
}
