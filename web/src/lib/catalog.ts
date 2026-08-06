import {
  Banknote,
  Bot,
  ChartColumn,
  Code,
  Crown,
  Eye,
  FileText,
  Globe,
  Hammer,
  House,
  MonitorPlay,
  Palette,
  Scale,
  Search,
  Server,
  Shield,
} from "lucide-react";

/**
 * Icons for the two catalogs (ADR-0021).
 *
 * Kept here rather than in the components that draw them: the hire dialog, the
 * org chart and the team proposal all render the same rows, and three private
 * copies of this map is how they came to disagree — two of them still named
 * `backend-developer` after the catalog had stopped shipping it.
 *
 * Slugs are *data*, so a user's own row simply falls back to `Bot` — the icon
 * map is not a place to enforce a closed set.
 */

/** The function an agent performs. */
export const FUNCTION_ICONS: Record<string, typeof Bot> = {
  "chief-executive": Crown,
  builder: Hammer,
  reviewer: Eye,
  researcher: Search,
  writer: FileText,
  analyst: ChartColumn,
};

/** The field it performs it in. */
export const DOMAIN_ICONS: Record<string, typeof Bot> = {
  general: Globe,
  software: Code,
  backend: Server,
  frontend: Palette,
  security: Shield,
  "media-av": MonitorPlay,
  "home-systems": House,
  finance: Banknote,
  legal: Scale,
};

export const FALLBACK_ICON = Bot;
