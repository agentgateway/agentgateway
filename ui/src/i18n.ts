import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en";
import zhCN from "./locales/zh-CN";

export const languageStorageKey = "agentgateway.language";
export const supportedLanguages = ["en", "zh-CN"] as const;
export type AppLanguage = (typeof supportedLanguages)[number];

function normalizeLanguage(
  value: string | null | undefined,
): AppLanguage | undefined {
  if (!value) return undefined;
  const normalized = value.replace("_", "-").toLowerCase();
  if (normalized === "zh" || normalized.startsWith("zh-cn")) return "zh-CN";
  if (normalized.startsWith("en")) return "en";
  return undefined;
}

function initialLanguage(): AppLanguage {
  const urlLanguage = new URLSearchParams(window.location.search).get("lang");
  const storedLanguage = window.localStorage.getItem(languageStorageKey);
  return (
    normalizeLanguage(urlLanguage) ??
    normalizeLanguage(storedLanguage) ??
    navigator.languages.map(normalizeLanguage).find(Boolean) ??
    "en"
  );
}

void i18n.use(initReactI18next).init({
  resources: { en, "zh-CN": zhCN },
  lng: initialLanguage(),
  fallbackLng: "en",
  supportedLngs: supportedLanguages,
  interpolation: { escapeValue: false },
  returnNull: false,
});

export function currentLanguage(): AppLanguage {
  return normalizeLanguage(i18n.resolvedLanguage ?? i18n.language) ?? "en";
}

export function setLanguage(language: AppLanguage) {
  window.localStorage.setItem(languageStorageKey, language);
  const url = new URL(window.location.href);
  url.searchParams.set("lang", language);
  window.history.replaceState(window.history.state, "", url);
  return i18n.changeLanguage(language);
}

type TranslationValue = string | number | boolean | null | undefined;
type TranslationValues = TranslationValue | readonly TranslationValue[];
type TranslationOptions = {
  count: number;
};

/**
 * Translate a semantic resource key without coupling utility modules to React
 * hooks.
 * Localized route wrappers subscribe to language changes and rerender without
 * remounting, so ordinary functions update while page-local state is retained.
 */
export function tr(
  key: string,
  values?: TranslationValues | TranslationOptions,
): string {
  const { count, replacements } = normalizeValues(values);
  const translated =
    count === undefined
      ? resourceValue(key)
      : i18n.t(key, { count, defaultValue: key });
  return interpolateValues(translated ?? key, replacements);
}

/** Translate runtime English text, such as JSON Schema descriptions. */
export function translateText(
  english: string,
  values?: TranslationValue | readonly TranslationValue[],
): string {
  const translated =
    resourceValue(`copy.${copyKeySegment(english)}`) ?? english;
  return interpolateValues(translated, values);
}

function resourceValue(key: string) {
  const current = i18n.getResource(currentLanguage(), "translation", key);
  if (typeof current === "string") return current;
  const fallback = i18n.getResource("en", "translation", key);
  return typeof fallback === "string" ? fallback : undefined;
}

function normalizeValues(values?: TranslationValues | TranslationOptions) {
  if (
    values !== null &&
    typeof values === "object" &&
    !Array.isArray(values) &&
    "count" in values
  ) {
    return { count: values.count, replacements: undefined };
  }
  return { count: undefined, replacements: values as TranslationValues };
}

function interpolateValues(translated: string, values?: TranslationValues) {
  const replacements = Array.isArray(values)
    ? values
    : values === undefined
      ? []
      : [values];
  let index = 0;
  return translated.replace(/\{\{value\}\}/g, () =>
    String(replacements[index++] ?? ""),
  );
}

function copyKeySegment(value: string) {
  const words = value
    .replace(/\{\{[^}]+\}\}/g, " value ")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .normalize("NFKD")
    .match(/[A-Za-z0-9]+/g);
  let key = (words?.length ? words : ["text", stableHash(value)])
    .map((word, index) => {
      const normalized = word.toLowerCase();
      return index === 0
        ? normalized
        : normalized.charAt(0).toUpperCase() + normalized.slice(1);
    })
    .join("");
  if (/^\d/.test(key)) key = `text${key}`;
  if (key.length > 96) key = `${key.slice(0, 80)}_${stableHash(value)}`;
  return key;
}

function stableHash(value: string) {
  let hash = 2166136261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

export default i18n;
