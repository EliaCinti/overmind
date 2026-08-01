import { createContext, useContext } from "react";
import type { LanguageCode } from "./api";

/**
 * The UI dictionary (M16, slice C).
 *
 * No library: this is a few hundred strings and a lookup, and a dependency
 * would buy plural rules and lazy bundles we do not need yet. What it must buy
 * is safety — `t()` is typed against the English dictionary, so a key that does
 * not exist, or a translation that drifts out of the shape, fails the build
 * rather than rendering `undefined` at someone.
 *
 * The language itself is *not* stored here: it belongs to the company, because
 * the server needs it to tell agents what to write (see `i18n.rs`). This module
 * only renders it.
 */
export const en = {
  nav: {
    chat: "Chat",
    board: "Board",
    meetings: "Meetings",
    org: "Org",
    hire: "Hire",
    newTask: "New task",
    newCompany: "+ New company…",
    noCompany: "No company",
    language: "Language",
    toggleTheme: "Toggle theme",
    inbox: "Inbox",
    unread: "{n} unread",
    waitingOnYou: "{n} waiting on your decision",
    nothingWaiting: "Nothing waiting on you.",
  },
  common: {
    approve: "Approve",
    reject: "Reject",
    decline: "Decline",
    cancel: "Cancel",
    markAllRead: "Mark all read",
    viewMeeting: "View meeting",
  },
  org: {
    you: "You",
    ownerLine: "Owner · everyone ultimately reports here",
    empty: "No agents yet. Hire your first one to build the org.",
    twoRoadsTitle: "{ceo} is your CEO, and the company is otherwise empty.",
    twoRoadsBody:
      "Tell {ceo} what you want to build and it will design the team — who to hire, in what role, reporting to whom — and put the chart in front of you before anyone is hired.",
    tellTheIdea: "Tell {ceo} the idea",
    orBuildYourself: "or build the team yourself",
  },
  proposal: {
    heading: "{ceo} proposes a team of {n}",
    hires: "Approving hires {n}. Nobody has been hired yet.",
    hiresAndSkips: "Approving hires {n} and skips {skipped}. Nobody has been hired yet.",
    allDropped: "You dropped everyone — put someone back, or decline the proposal.",
    hire: "Hire {n}",
    hiring: "Hiring…",
    skipped: "Skipped. Anyone reporting to them will report to the CEO instead.",
    skipOne: "Skip {name}",
    putBack: "Put {name} back",
  },
} as const;

/** A translation must have exactly the shape of the English one. */
export type Dictionary = {
  [S in keyof typeof en]: { [K in keyof (typeof en)[S]]: string };
};

export const it: Dictionary = {
  nav: {
    chat: "Chat",
    board: "Bacheca",
    meetings: "Riunioni",
    org: "Organico",
    hire: "Assumi",
    newTask: "Nuovo task",
    newCompany: "+ Nuova azienda…",
    noCompany: "Nessuna azienda",
    language: "Lingua",
    toggleTheme: "Cambia tema",
    inbox: "Notifiche",
    unread: "{n} da leggere",
    waitingOnYou: "{n} in attesa di una tua decisione",
    nothingWaiting: "Non c'è nulla in attesa.",
  },
  common: {
    approve: "Approva",
    reject: "Rifiuta",
    decline: "Rifiuta",
    cancel: "Annulla",
    markAllRead: "Segna tutto come letto",
    viewMeeting: "Vedi la riunione",
  },
  org: {
    you: "Tu",
    ownerLine: "Proprietario · tutti rispondono qui, in ultima istanza",
    empty: "Nessun agente. Assumi il primo per costruire l'organico.",
    twoRoadsTitle: "{ceo} è il tuo CEO, e l'azienda per ora è vuota.",
    twoRoadsBody:
      "Racconta a {ceo} cosa vuoi costruire: disegnerà la squadra — chi assumere, con che ruolo, chi risponde a chi — e ti metterà davanti l'organigramma prima che venga assunto qualcuno.",
    tellTheIdea: "Racconta l'idea a {ceo}",
    orBuildYourself: "oppure costruisci la squadra tu",
  },
  proposal: {
    heading: "{ceo} propone una squadra di {n}",
    hires: "Approvando ne assumi {n}. Non è ancora stato assunto nessuno.",
    hiresAndSkips:
      "Approvando ne assumi {n} e ne scarti {skipped}. Non è ancora stato assunto nessuno.",
    allDropped: "Li hai scartati tutti — rimettine qualcuno, oppure rifiuta la proposta.",
    hire: "Assumi {n}",
    hiring: "Assunzione…",
    skipped: "Scartato. Chi rispondeva a lui risponderà al CEO.",
    skipOne: "Scarta {name}",
    putBack: "Rimetti {name}",
  },
};

const DICTIONARIES: Record<LanguageCode, Dictionary> = { en, it };

/** `t("org.tellTheIdea", { ceo: "Aria" })` */
export type Translate = (key: TranslationKey, vars?: Record<string, string | number>) => string;

type TranslationKey = {
  [S in keyof typeof en]: `${S & string}.${keyof (typeof en)[S] & string}`;
}[keyof typeof en];

export const LanguageContext = createContext<LanguageCode>("en");

export function useLanguage(): LanguageCode {
  return useContext(LanguageContext);
}

/** The translator for the current language. */
export function useT(): Translate {
  const language = useLanguage();
  const dict = DICTIONARIES[language] ?? en;
  return (key, vars) => {
    const [section, name] = key.split(".") as [keyof Dictionary, string];
    const table = dict[section] as Record<string, string> | undefined;
    // Fall back to English rather than to the key: a missing Italian string
    // should read as English, not as `proposal.heading`.
    const template =
      table?.[name] ?? (en[section] as Record<string, string> | undefined)?.[name] ?? key;
    if (!vars) return template;
    return Object.entries(vars).reduce(
      (out, [k, v]) => out.replaceAll(`{${k}}`, String(v)),
      template,
    );
  };
}

/** How dates and money read in this language. */
export function useFormats() {
  const language = useLanguage();
  const locale = language === "it" ? "it-IT" : "en-US";
  // The product is priced in euro; showing dollars was a leftover of the
  // dollar-shaped `_cents` field name, not a decision.
  const money = new Intl.NumberFormat(locale, { style: "currency", currency: "EUR" });
  return {
    locale,
    formatCents: (cents: number) => money.format(cents / 100),
  };
}
