import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/** Merge Tailwind classes with conflict resolution. */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

// Money and relative time used to live here, hard-coded to US English. They
// moved to `useFormats()` in i18n.ts, where they can read the company's
// language — see the note there on why the platform words them and we do not.
