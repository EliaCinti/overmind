import type { LanguageCode } from "../lib/api";
import { LanguageContext } from "../lib/i18n";

/** Makes the company's language available to `useT()` anywhere below. */
export function LanguageProvider({
  language,
  children,
}: {
  language: LanguageCode;
  children: React.ReactNode;
}) {
  return <LanguageContext.Provider value={language}>{children}</LanguageContext.Provider>;
}
