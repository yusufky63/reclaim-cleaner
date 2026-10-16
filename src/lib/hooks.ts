import { useTranslation } from "react-i18next";
import { localeOf } from "../i18n/languages";
import { formatBytes } from "./format";

/** BCP 47 locale for Intl formatting, following the UI language. */
export function useLocale(): string {
  const { i18n } = useTranslation();
  return localeOf(i18n.language);
}

export function useBytes() {
  const locale = useLocale();
  return (n: number) => formatBytes(n, locale);
}
