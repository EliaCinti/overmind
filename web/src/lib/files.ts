import {
  FileArchive,
  FileAudio,
  FileCode2,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo,
  File as FileIcon,
} from "lucide-react";

/**
 * How a file is presented (M17). Not components — they live here so the
 * components that use them stay hot-reloadable.
 */

/** A size, in the reader's number format: `2,4 MB` in Italian, `2.4 MB` in English. */
export function formatBytes(bytes: number, locale: string): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // One decimal below 10, none above — the extra digit stops mattering
  // exactly where the number gets long.
  const digits = value < 10 ? 1 : 0;
  return `${value.toLocaleString(locale, { maximumFractionDigits: digits })} ${units[unit]}`;
}

/** `research/sources.csv` → the folder and the file, shown differently. */
export function splitPath(path: string): { dir: string; name: string } {
  const i = path.lastIndexOf("/");
  return i === -1 ? { dir: "", name: path } : { dir: path.slice(0, i), name: path.slice(i + 1) };
}

/**
 * One glyph per family, so a list of files is scannable before it is read.
 * Deliberately coarse: what matters at a glance is "document / data / picture /
 * code / archive", not the exact format.
 */
export function iconFor(mime: string): typeof FileIcon {
  if (mime.startsWith("image/")) return FileImage;
  if (mime.startsWith("audio/")) return FileAudio;
  if (mime.startsWith("video/")) return FileVideo;
  if (mime === "text/csv" || mime === "text/tab-separated-values" || mime.includes("spreadsheet"))
    return FileSpreadsheet;
  if (
    mime.startsWith("text/x-") ||
    mime === "text/typescript" ||
    mime === "text/javascript" ||
    mime === "text/css" ||
    mime === "application/json" ||
    mime === "application/sql" ||
    mime === "application/yaml" ||
    mime === "application/toml" ||
    mime === "application/xml"
  )
    return FileCode2;
  if (mime === "application/zip" || mime === "application/gzip" || mime === "application/x-tar")
    return FileArchive;
  if (mime.startsWith("text/") || mime === "application/pdf" || mime.includes("document"))
    return FileText;
  return FileIcon;
}
