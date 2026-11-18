//! Interface translations.
//!
//! The English text *is* the key, so code stays readable:
//!
//! ```ignore
//! ui.label(t!("Sign in"));
//! ui.label(t!("Playing {game}", game = name));
//! ```
//!
//! Other languages live in `locales/<code>.toml` as `"English" = "Translation"`.
//! Missing entries fall back to English. To add a language, add a file and a
//! [`Lang`] variant. A test checks that every `t!` string in the app has an
//! entry in every language.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Russian,
    Chinese,
    Spanish,
    Portuguese,
    German,
    French,
}

impl Lang {
    pub const ALL: [Lang; 7] = [Lang::English, Lang::Russian, Lang::Chinese, Lang::Spanish, Lang::Portuguese, Lang::German, Lang::French];

    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Russian => "ru",
            Lang::Chinese => "zh",
            Lang::Spanish => "es",
            Lang::Portuguese => "pt",
            Lang::German => "de",
            Lang::French => "fr",
        }
    }
    /// The language's own name.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Russian => "Русский",
            Lang::Chinese => "简体中文",
            Lang::Spanish => "Español",
            Lang::Portuguese => "Português (Brasil)",
            Lang::German => "Deutsch",
            Lang::French => "Français",
        }
    }
    /// Name Steam's store API expects (`l=` parameter).
    pub fn steam_name(self) -> &'static str {
        match self {
            Lang::English => "english",
            Lang::Russian => "russian",
            Lang::Chinese => "schinese",
            Lang::Spanish => "spanish",
            Lang::Portuguese => "brazilian",
            Lang::German => "german",
            Lang::French => "french",
        }
    }
    pub fn from_code(code: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| code.to_lowercase().starts_with(l.code()))
    }
    /// The system language if we have it, otherwise English.
    pub fn detect() -> Lang {
        sys_locale::get_locale().and_then(|l| Lang::from_code(&l)).unwrap_or(Lang::English)
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn language() -> Lang {
    Lang::ALL.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or(Lang::English)
}

fn table(lang: Lang) -> Option<&'static HashMap<String, &'static str>> {
    type Table = OnceLock<HashMap<String, &'static str>>;
    static RU: Table = OnceLock::new();
    static ZH: Table = OnceLock::new();
    static ES: Table = OnceLock::new();
    static PT: Table = OnceLock::new();
    static DE: Table = OnceLock::new();
    static FR: Table = OnceLock::new();
    Some(match lang {
        Lang::English => return None,
        Lang::Russian => RU.get_or_init(|| load(include_str!("../locales/ru.toml"))),
        Lang::Chinese => ZH.get_or_init(|| load(include_str!("../locales/zh.toml"))),
        Lang::Spanish => ES.get_or_init(|| load(include_str!("../locales/es.toml"))),
        Lang::Portuguese => PT.get_or_init(|| load(include_str!("../locales/pt.toml"))),
        Lang::German => DE.get_or_init(|| load(include_str!("../locales/de.toml"))),
        Lang::French => FR.get_or_init(|| load(include_str!("../locales/fr.toml"))),
    })
}

fn load(src: &str) -> HashMap<String, &'static str> {
    let parsed: toml::Table = toml::from_str(src).expect("valid locale file");
    parsed
        .into_iter()
        .filter_map(|(k, v)| Some((k, &*Box::leak(v.as_str()?.to_string().into_boxed_str()))))
        .collect()
}

/// Translate a fixed English string.
/// Keys can carry a context after `|` when one English word needs
/// different translations ("Recommended|review" vs the store's
/// "Recommended"); the context is never shown.
pub fn tr(english: &'static str) -> &'static str {
    table(language()).and_then(|t| t.get(english).copied()).unwrap_or_else(|| english.split_once('|').map_or(english, |(text, _)| text))
}

/// Translate a string only known at runtime (e.g. an error from the engine).
pub fn tr_dyn(english: &str) -> String {
    table(language()).and_then(|t| t.get(english).map(|s| s.to_string())).unwrap_or_else(|| english.to_string())
}

/// Translate and fill `{name}` placeholders.
pub fn trf(english: &'static str, args: &[(&str, String)]) -> String {
    let mut out = tr(english).to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[macro_export]
macro_rules! t {
    ($s:literal) => {
        $crate::tr($s)
    };
    ($s:literal, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::trf($s, &[$((stringify!($k), ($v).to_string())),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    // The language is global: only this test changes it (tests run in
    // parallel, so a second one switching it made this one fail at random).
    #[test]
    fn falls_back_and_formats() {
        set_language(Lang::English);
        assert_eq!(t!("Sign in"), "Sign in");
        // A context after "|" is never shown.
        assert_eq!(tr("Recommended|review"), "Recommended");
        assert_eq!(t!("Playing {game}", game = "Dota 2"), "Playing Dota 2");
        set_language(Lang::Russian);
        assert_eq!(t!("Sign in"), "Войти");
        assert_eq!(t!("Playing {game}", game = "Dota 2"), "Играет в Dota 2");
        set_language(Lang::English);
    }

    /// Every `t!("...")` in the app must have a translation in every language.
    #[test]
    fn every_language_covers_all_strings() {
        let tables: Vec<(Lang, &HashMap<String, &str>)> = Lang::ALL.into_iter().filter_map(|l| Some((l, table(l)?))).collect();
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut missing = Vec::new();
        let mut stack = vec![base.join("../../apps/desktop/src"), base.join("../../apps/installer/src"), base.join("../ui/src"), base.join("../video/src")];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let src = std::fs::read_to_string(&p).unwrap();
                let parts: Vec<&str> = src.split("t!(\"").collect();
                for (i, part) in parts.iter().enumerate().skip(1) {
                    // Skip `format!(`, `print!(` and similar.
                    if parts[i - 1].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(end) = part.find('"') else { continue };
                    let key = &part[..end];
                    for (lang, t) in &tables {
                        let entry = format!("{}: \"{key}\"", lang.code());
                        if !t.contains_key(key) && !missing.contains(&entry) {
                            missing.push(entry);
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing translations:\n{}", missing.join("\n"));
    }

    /// Every language has the same strings as Russian (the most complete),
    /// and keeps each `{placeholder}`.
    #[test]
    fn languages_match_and_keep_placeholders() {
        let ru = table(Lang::Russian).unwrap();
        let holes = |s: &str| {
            let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).filter(|n| n.chars().all(|c| c.is_ascii_lowercase() || c == '_')).collect();
            v.sort();
            v
        };
        for lang in Lang::ALL {
            let Some(t) = table(lang) else { continue };
            for key in ru.keys() {
                let v = t.get(key).unwrap_or_else(|| panic!("{}: missing \"{key}\"", lang.code()));
                assert_eq!(holes(key), holes(v), "{}: placeholders differ in \"{key}\"", lang.code());
            }
            assert_eq!(t.len(), ru.len(), "{}: extra strings", lang.code());
        }
    }

    #[test]
    fn picks_languages_from_codes() {
        assert_eq!(Lang::from_code("zh-CN"), Some(Lang::Chinese));
        assert_eq!(Lang::from_code("es-MX"), Some(Lang::Spanish));
        assert_eq!(Lang::from_code("pt_BR"), Some(Lang::Portuguese));
        assert_eq!(Lang::from_code("de-AT"), Some(Lang::German));
        assert_eq!(Lang::from_code("fr-CA"), Some(Lang::French));
    }
}
