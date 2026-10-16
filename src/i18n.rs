//! Translations for the strings Rust builds itself.
//!
//! Slint's bundled translations (see `build.rs`) only rewrite `@tr(...)` inside
//! `.slint` files: the compiler turns each of those into a lookup in a table it
//! generates, and there is no public API to run an arbitrary msgid through the same
//! catalog. Everything the engine and the transfer flows format — statuses, toasts,
//! history rows, error messages — therefore needs its own copy of the catalogs,
//! which `build.rs` generates from the very same `.po` files.
//!
//! The two sides are kept in step by convention: a message that appears in both is
//! written identically (same English msgid, `{}` and `{n}` placeholders) and lives in
//! one `.po` entry.

use std::sync::atomic::{AtomicUsize, Ordering};

mod table {
    include!(concat!(env!("OUT_DIR"), "/i18n_table.rs"));
}

/// Index into `table::TABLES`; 0 is "no translation", the English msgids.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The language codes this build bundles, for the Settings page.
pub fn bundled_languages() -> &'static [&'static str] {
    table::LANGUAGES
}

/// The language code a Settings value resolves to: `""` for the English msgids.
pub fn resolve(setting: &str) -> &'static str {
    match setting {
        "" | "system" => system_language(),
        "en" => "",
        other => table::LANGUAGES
            .iter()
            .copied()
            .find(|l| *l == other)
            .unwrap_or(""),
    }
}

/// Pick and remember the language for both Slint's `@tr(...)` strings and [`tr`].
/// Call again whenever the setting changes; call after the first component exists,
/// because Slint only learns about the bundled catalogs from the generated code.
pub fn apply(setting: &str) {
    let language = resolve(setting);
    let selected = if language.is_empty() {
        let _ = slint::select_bundled_translation("");
        ""
    } else if slint::select_bundled_translation(language).is_ok() {
        language
    } else {
        // A bundled folder Slint did not pick up; showing English beats showing
        // another language's strings.
        tracing::warn!("no bundled translation for {language}");
        let _ = slint::select_bundled_translation("");
        ""
    };
    let index = table::LANGUAGES
        .iter()
        .position(|l| *l == selected)
        .map_or(0, |i| i + 1);
    CURRENT.store(index, Ordering::Relaxed);
}

/// Translate `msgid` (English source text, `{}`/`{n}` placeholders) for the current
/// language.
pub fn tr(msgid: &str) -> String {
    render(source(msgid), None, &[])
}

/// [`tr`] with the `{}`/`{0}`, `{1}` placeholders filled from `args`.
pub fn tr_args(msgid: &str, args: &[&str]) -> String {
    render(source(msgid), None, args)
}

/// Translate a plural: `msgid` is the singular, `msgid_plural` the same message for
/// other counts, and `n` both selects the form (per the language's plural rule) and
/// fills `{n}`. Further `{}` placeholders come from `args`.
pub fn trn(msgid: &str, msgid_plural: &str, n: i64, args: &[&str]) -> String {
    let Some(forms) = lookup(msgid).or_else(|| lookup(msgid_plural)) else {
        return render(if n == 1 { msgid } else { msgid_plural }, Some(n), args);
    };
    let form = forms
        .get(plural_index(n))
        .or_else(|| forms.first())
        .copied();
    render(form.unwrap_or(msgid), Some(n), args)
}

/// The first form of `msgid` in the current language, or the English source text.
fn source(msgid: &str) -> &str {
    lookup(msgid)
        .and_then(|forms| forms.first().copied())
        .unwrap_or(msgid)
}

fn lookup(msgid: &str) -> Option<&'static [&'static str]> {
    let index = CURRENT.load(Ordering::Relaxed);
    let table = table::TABLES.get(index.checked_sub(1)?)?;
    table
        .binary_search_by_key(&msgid, |(msgid, _)| *msgid)
        .ok()
        .map(|i| table[i].1)
}

/// Chinese, Japanese and Korean have a single plural form (like most languages in
/// Slint's catalog set); everything else uses the English one-out-of-many rule.
fn plural_index(n: i64) -> usize {
    let language = table::LANGUAGES
        .get(CURRENT.load(Ordering::Relaxed).wrapping_sub(1))
        .copied()
        .unwrap_or("");
    let base = language.split(['-', '_', '@']).next().unwrap_or(language);
    match base {
        "zh" | "ja" | "ko" | "th" | "vi" => 0,
        _ => usize::from(n != 1),
    }
}

/// `{}` in order, `{0}`/`{1}` by position, `{n}` the plural count. `{{`/`}}` escape.
fn render(template: &str, n: Option<i64>, args: &[&str]) -> String {
    // Park the escaped braces out of the way, then restore them at the end.
    let template = template.replace("{{", "\u{1}").replace("}}", "\u{2}");
    let mut out = String::with_capacity(template.len() + 8);
    let mut rest = template.as_str();
    let mut next = 0usize;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        match rest[open + 1..].find('}') {
            None => {
                out.push_str(&rest[open..]);
                return out;
            }
            Some(close) => {
                let close = open + 1 + close;
                let key = &rest[open + 1..close];
                match key {
                    "" => {
                        match args.get(next) {
                            Some(arg) => out.push_str(arg),
                            // No argument for it: leave the placeholder alone rather
                            // than silently swallowing it.
                            None => out.push_str(&rest[open..=close]),
                        }
                        next += 1;
                    }
                    "n" => match n {
                        Some(n) => out.push_str(&n.to_string()),
                        None => out.push_str(&rest[open..=close]),
                    },
                    _ => match key.parse::<usize>() {
                        Ok(i) => {
                            if let Some(arg) = args.get(i) {
                                out.push_str(arg);
                            }
                        }
                        Err(_) => out.push_str(&rest[open..=close]),
                    },
                }
                rest = &rest[close + 1..];
            }
        }
    }
    out.push_str(rest);
    out.replace('\u{1}', "{").replace('\u{2}', "}")
}

/// The system locale, reduced to one of the bundled languages.
fn system_language() -> &'static str {
    let Some(locale) = sys_locale::get_locale() else {
        return "";
    };
    if let Some(language) = table::LANGUAGES.iter().copied().find(|l| *l == locale) {
        return language;
    }
    // zh-Hans-CN / zh_CN / zh-TW@… all reduce to the language part.
    fn base(language: &str) -> &str {
        language.split(['-', '_', '@']).next().unwrap_or(language)
    }
    let wanted = base(&locale);
    table::LANGUAGES
        .iter()
        .copied()
        .find(|l| base(l) == wanted)
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_msgids_pass_through() {
        assert_eq!(
            tr("No such message in the catalog"),
            "No such message in the catalog"
        );
    }

    #[test]
    fn placeholders_are_filled_in_order_and_by_index() {
        assert_eq!(render("Sent to {}", None, &["Laptop"]), "Sent to Laptop");
        assert_eq!(render("{0} and {1}", None, &["a", "b"]), "a and b");
        assert_eq!(
            render("Waiting for {} to come online", None, &["Phone"]),
            "Waiting for Phone to come online"
        );
        assert_eq!(render("{} items", None, &["3"]), "3 items");
        // A placeholder without an argument is left alone rather than dropped.
        assert_eq!(render("{} of {}", None, &["1"]), "1 of {}");
        assert_eq!(render("{n} files", Some(2), &[]), "2 files");
        assert_eq!(render("{{quoted}}", None, &[]), "{quoted}");
    }

    #[test]
    fn english_is_the_msgid() {
        assert_eq!(resolve("en"), "");
        assert_eq!(resolve(""), resolve("system"));
    }
}
