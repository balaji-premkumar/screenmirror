//! Event codes and locale catalogs shared by the backend and its interfaces.
//!
//! # Why the backend does not emit sentences
//!
//! Log lines from the backend are not developer-only diagnostics — they are
//! rendered in the desktop app's "Diagnostic Stream" panel, so a user reads
//! them. When those lines were built with `format!` at the call site, the
//! English was compiled into the library and crossed the FFI boundary as
//! finished prose. Translating it would have meant translating inside Rust, at
//! a layer that has no idea what the user's language is.
//!
//! A call site now names a stable [`codes`] constant and supplies parameters:
//!
//! ```
//! # use mirror_i18n::{codes, Event};
//! let event = Event::new(codes::USB_STREAMING_CONFIG_SENT).with("bytes", 412);
//! assert_eq!(event.render_en(), "Settings sent (412 bytes).");
//! ```
//!
//! The code and its parameters travel to the interface, which looks the
//! wording up in the locale the *user* chose. [`Event::render_en`] stays
//! available so the on-disk log file and any interface that has not been
//! localised yet still read as English.
//!
//! # One catalog, several consumers
//!
//! [`catalog/en.json`](../../catalog/en.json) is the single source of truth.
//! This crate embeds it with `include_str!`; the desktop UI imports the same
//! file directly. Neither can drift from the other because there is only one
//! file. A new locale is that file translated — no Rust change at all.

#![forbid(unsafe_code)]

pub mod codes;

use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::BTreeMap;

/// The English catalog, embedded at compile time.
///
/// Exposed so a consumer that wants the raw JSON — a translation tool, say —
/// does not have to locate the file on disk.
pub const CATALOG_EN_JSON: &str = include_str!("../catalog/en.json");

/// One entry in a locale catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct CatalogEntry {
    /// Severity: `INFO`, `SUCCESS`, `WARN`, `ERROR` or `FATAL`.
    ///
    /// It lives in the catalog rather than at the call site so that a code's
    /// severity is decided in one place — the same place its wording is.
    pub level: String,
    /// Which subsystem the event came from, e.g. `USB`, `DECODER`.
    pub component: String,
    /// The activity within that subsystem, e.g. `handshake`, `pipeline`.
    pub action: String,
    /// The wording, with `{name}` placeholders filled from an event's params.
    pub message: String,
}

/// A parsed locale catalog.
#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    /// BCP 47 language tag, e.g. `en`.
    pub locale: String,
    /// The language's name in that language.
    pub name: String,
    /// Every entry, keyed by event code.
    pub entries: BTreeMap<String, CatalogEntry>,
}

static EN: Lazy<Catalog> = Lazy::new(|| {
    serde_json::from_str(CATALOG_EN_JSON)
        .expect("catalog/en.json is embedded at compile time and must parse")
});

/// The English catalog.
#[must_use]
pub fn catalog_en() -> &'static Catalog {
    &EN
}

/// Looks up an entry in the English catalog.
#[must_use]
pub fn entry_en(code: &str) -> Option<&'static CatalogEntry> {
    EN.entries.get(code)
}

/// An event ready to be logged: a code plus the values its wording needs.
///
/// Parameters are stringified when they are added, which keeps the type simple
/// and means the interface receives values it can display without knowing
/// their original Rust type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    code: String,
    params: Vec<(String, String)>,
}

impl Event {
    /// Starts an event for `code`, which should come from [`codes`].
    #[must_use]
    pub fn new(code: &str) -> Self {
        Event {
            code: code.to_string(),
            params: Vec::new(),
        }
    }

    /// Adds a parameter. `name` matches a `{name}` placeholder in the catalog.
    #[must_use]
    pub fn with(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.params.push((name.to_string(), value.to_string()));
        self
    }

    /// The event code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The parameters, in the order they were added.
    #[must_use]
    pub fn params(&self) -> &[(String, String)] {
        &self.params
    }

    /// Severity from the English catalog, or `INFO` for an unknown code.
    #[must_use]
    pub fn level(&self) -> &str {
        entry_en(&self.code).map_or("INFO", |e| e.level.as_str())
    }

    /// Component from the English catalog, or `UNKNOWN` for an unknown code.
    #[must_use]
    pub fn component(&self) -> &str {
        entry_en(&self.code).map_or("UNKNOWN", |e| e.component.as_str())
    }

    /// Action from the English catalog, or an empty string for an unknown code.
    #[must_use]
    pub fn action(&self) -> &str {
        entry_en(&self.code).map_or("", |e| e.action.as_str())
    }

    /// Renders the English wording with parameters substituted.
    ///
    /// An unknown code renders as the code itself followed by its parameters,
    /// so a missing catalog entry degrades to something still diagnosable
    /// rather than to an empty line.
    #[must_use]
    pub fn render_en(&self) -> String {
        match entry_en(&self.code) {
            Some(entry) => render(&entry.message, &self.params),
            None => {
                let mut out = self.code.clone();
                for (k, v) in &self.params {
                    out.push_str(&format!(" {k}={v}"));
                }
                out
            }
        }
    }
}

/// Substitutes `{name}` placeholders in `template` from `params`.
///
/// A placeholder with no matching parameter is left in place. Blanking it
/// would hide the omission; leaving `{error}` visible in a log line makes the
/// bug obvious to whoever reads it.
#[must_use]
pub fn render(template: &str, params: &[(String, String)]) -> String {
    if !template.contains('{') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match params.iter().find(|(k, _)| k == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                // Unterminated brace: emit the remainder verbatim.
                out.push('{');
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_declared_code_has_an_english_entry() {
        let catalog: BTreeSet<&str> = EN.entries.keys().map(String::as_str).collect();
        let declared: BTreeSet<&str> = codes::ALL.iter().copied().collect();

        let untranslated: Vec<&&str> = declared.difference(&catalog).collect();
        assert!(
            untranslated.is_empty(),
            "these codes are declared in codes.rs but missing from catalog/en.json: {untranslated:?}"
        );
    }

    #[test]
    fn the_english_catalog_has_no_orphan_entries() {
        let catalog: BTreeSet<&str> = EN.entries.keys().map(String::as_str).collect();
        let declared: BTreeSet<&str> = codes::ALL.iter().copied().collect();

        let orphans: Vec<&&str> = catalog.difference(&declared).collect();
        assert!(
            orphans.is_empty(),
            "these entries are in catalog/en.json but no code declares them, so nothing can emit them: {orphans:?}"
        );
    }

    #[test]
    fn every_code_is_unique() {
        let unique: BTreeSet<&str> = codes::ALL.iter().copied().collect();
        assert_eq!(
            unique.len(),
            codes::ALL.len(),
            "two constants in codes.rs share a code string"
        );
    }

    #[test]
    fn every_entry_declares_a_known_level() {
        const LEVELS: [&str; 5] = ["INFO", "SUCCESS", "WARN", "ERROR", "FATAL"];
        for (code, entry) in &EN.entries {
            assert!(
                LEVELS.contains(&entry.level.as_str()),
                "{code} has level {:?}, which the UI does not style",
                entry.level
            );
            assert!(!entry.component.is_empty(), "{code} has no component");
            assert!(!entry.message.is_empty(), "{code} has no message");
        }
    }

    #[test]
    fn renders_parameters_into_placeholders() {
        let e = Event::new(codes::USB_STREAMING_CONFIG_SENT).with("bytes", 412);
        assert_eq!(e.render_en(), "Settings sent (412 bytes).");
        assert_eq!(e.level(), "SUCCESS");
        assert_eq!(e.component(), "USB");
        assert_eq!(e.action(), "streaming");
    }

    #[test]
    fn renders_repeated_and_multiple_placeholders() {
        assert_eq!(
            render(
                "{a} then {b} then {a}",
                &[("a".into(), "1".into()), ("b".into(), "2".into())]
            ),
            "1 then 2 then 1"
        );
    }

    #[test]
    fn a_missing_parameter_stays_visible() {
        // Silently dropping it would hide a call site that forgot an argument.
        assert_eq!(render("open failed: {error}", &[]), "open failed: {error}");
    }

    #[test]
    fn unterminated_and_absent_braces_do_not_panic() {
        assert_eq!(render("no placeholders", &[]), "no placeholders");
        assert_eq!(render("dangling {oops", &[]), "dangling {oops");
        assert_eq!(render("{}", &[]), "{}");
    }

    #[test]
    fn an_unknown_code_degrades_to_something_diagnosable() {
        let e = Event::new("not.a.real.code").with("x", 7);
        assert_eq!(e.render_en(), "not.a.real.code x=7");
        assert_eq!(e.level(), "INFO");
        assert_eq!(e.component(), "UNKNOWN");
    }

    #[test]
    fn every_placeholder_in_the_catalog_is_lowercase_and_simple() {
        // The UI substitutes on exact name match, so a stray space or capital
        // in a template silently produces an unsubstituted placeholder.
        for (code, entry) in &EN.entries {
            let mut rest = entry.message.as_str();
            while let Some(open) = rest.find('{') {
                let after = &rest[open + 1..];
                let close = after
                    .find('}')
                    .unwrap_or_else(|| panic!("{code} has an unterminated placeholder"));
                let name = &after[..close];
                assert!(
                    !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                    "{code} has placeholder {{{name}}}; use lower_snake_case"
                );
                rest = &after[close + 1..];
            }
        }
    }
}
