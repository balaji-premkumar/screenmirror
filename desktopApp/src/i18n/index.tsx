/**
 * Translation for the desktop interface.
 *
 * Two catalogs feed it, and they are deliberately separate:
 *
 *   * `locales/*.json` — strings this UI owns (buttons, headings, tooltips).
 *   * `packages/mirror-i18n/catalog/*.json` — wording for backend events,
 *     shared with the Rust backend so both sides key off the same codes.
 *
 * Adding a language is two JSON files and one line in {@link LOCALES}. No
 * component changes, because no component contains English.
 */

import { createContext, useCallback, useContext, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import en from './locales/en.json';

/** Values substituted into `{name}` placeholders. */
export type Params = Record<string, string | number>;

/** A flat key → template map. */
export type Catalog = Record<string, string>;

/**
 * Every locale this build ships.
 *
 * To add one: translate `locales/en.json` and
 * `packages/mirror-i18n/catalog/en.json`, then add both here and in
 * `events.ts`. The `Locale` type widens automatically.
 */
export const LOCALES = {
  en: { name: 'English', catalog: en as Catalog },
} as const;

export type Locale = keyof typeof LOCALES;

/** Falls back to this whenever a key is missing from the active locale. */
export const FALLBACK_LOCALE: Locale = 'en';

/**
 * Substitutes `{name}` placeholders.
 *
 * A placeholder with no matching parameter is left visible rather than
 * blanked: `{error}` in the UI is an obvious bug report, an empty gap is not.
 * This matches `mirror_i18n::render` on the Rust side.
 */
export function interpolate(template: string, params?: Params): string {
  if (!params || !template.includes('{')) return template;
  return template.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}

/**
 * Resolves `key` in `locale`, falling back to English and then to the key.
 *
 * Returning the key rather than an empty string means a missing translation
 * shows up as `settings.bitrate` in the UI — visibly wrong, and immediately
 * traceable to the catalog entry that needs writing.
 */
export function translate(locale: Locale, key: string, params?: Params): string {
  const template =
    LOCALES[locale]?.catalog[key] ?? LOCALES[FALLBACK_LOCALE].catalog[key] ?? key;
  return interpolate(template, params);
}

interface I18nValue {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (key: string, params?: Params) => string;
}

const I18nContext = createContext<I18nValue | null>(null);

/** Picks the best available locale for the browser's language preferences. */
export function detectLocale(
  preferred: readonly string[] = typeof navigator === 'undefined' ? [] : navigator.languages,
): Locale {
  for (const tag of preferred) {
    const base = tag.split('-')[0];
    if (base in LOCALES) return base as Locale;
  }
  return FALLBACK_LOCALE;
}

export function I18nProvider({
  children,
  initialLocale,
}: {
  children: ReactNode;
  initialLocale?: Locale;
}) {
  const [locale, setLocale] = useState<Locale>(() => initialLocale ?? detectLocale());

  const t = useCallback(
    (key: string, params?: Params) => translate(locale, key, params),
    [locale],
  );

  const value = useMemo(() => ({ locale, setLocale, t }), [locale, t]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

/**
 * Access to the active locale's `t`.
 *
 * Throws outside a provider rather than silently falling back to English,
 * because a component rendering untranslated text is the exact failure this
 * module exists to prevent.
 */
export function useI18n(): I18nValue {
  const value = useContext(I18nContext);
  if (!value) {
    throw new Error('useI18n must be used inside <I18nProvider>');
  }
  return value;
}

/** Shorthand for `useI18n().t`. */
export function useT() {
  return useI18n().t;
}
