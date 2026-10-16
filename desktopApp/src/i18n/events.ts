/**
 * Renders backend log events in the user's language.
 *
 * The backend sends a stable `code` plus `params` — never a finished sentence
 * — precisely so this layer can choose the wording. The catalog it reads is
 * the *same file* the Rust crate embeds, imported straight from
 * `packages/mirror-i18n/`. There is no copy to keep in sync because there is
 * no copy.
 */

import enEvents from '@catalog/en.json';
import type { LogEntry } from '@/types';
import { FALLBACK_LOCALE, interpolate, type Locale } from './index';

/** One entry in a shared catalog. Mirrors `mirror_i18n::CatalogEntry`. */
interface EventEntry {
  level: string;
  component: string;
  action: string;
  message: string;
}

interface EventCatalog {
  locale: string;
  name: string;
  entries: Record<string, EventEntry>;
}

/**
 * Event catalogs by locale.
 *
 * Only English exists today. A new language is a translated copy of
 * `packages/mirror-i18n/catalog/en.json` added here — the Rust side needs no
 * change, because it only ever emits codes.
 */
const EVENT_CATALOGS: Partial<Record<Locale, EventCatalog>> = {
  en: enEvents as EventCatalog,
};

/**
 * The text to show for a log entry.
 *
 * Resolution order:
 *
 *   1. the active locale's catalog,
 *   2. the English catalog,
 *   3. `entry.message`, which the backend already rendered in English.
 *
 * Step 3 matters more than it looks: it means a backend newer than the UI —
 * one emitting a code this build has never heard of — still shows something
 * readable instead of a bare identifier.
 */
export function renderLogMessage(entry: LogEntry, locale: Locale): string {
  const template =
    EVENT_CATALOGS[locale]?.entries[entry.code]?.message ??
    EVENT_CATALOGS[FALLBACK_LOCALE]?.entries[entry.code]?.message;

  if (!template) return entry.message;
  return interpolate(template, entry.params);
}

/** Whether this build knows how to translate `code`. */
export function hasEventTranslation(code: string, locale: Locale): boolean {
  return Boolean(EVENT_CATALOGS[locale]?.entries[code]);
}
