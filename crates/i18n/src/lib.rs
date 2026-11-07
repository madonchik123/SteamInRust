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
//! [`Lang`] variant. A test checks that every `t!` string in the app has a
//! Russian entry.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Russian,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::English, Lang::Russian];

    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Russian => "ru",
        }
    }
    /// The language's own name.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Russian => "Русский",
        }
    }
    /// Name Steam's store API expects (`l=` parameter).
    pub fn steam_name(self) -> &'static str {
        match self {
            Lang::English => "english",
            Lang::Russian => "russian",
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
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Russian,
        _ => Lang::English,
    }
}

fn table(lang: Lang) -> Option<&'static HashMap<String, &'static str>> {
    static RU: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    match lang {
        Lang::English => None,
        Lang::Russian => Some(RU.get_or_init(|| load(include_str!("../locales/ru.toml")))),
    }
}

fn load(src: &str) -> HashMap<String, &'static str> {
    let parsed: toml::Table = toml::from_str(src).expect("valid locale file");
    parsed
        .into_iter()
        .filter_map(|(k, v)| Some((k, &*Box::leak(v.as_str()?.to_string().into_boxed_str()))))
        .collect()
}

/// Translate a fixed English string.
pub fn tr(english: &'static str) -> &'static str {
    table(language()).and_then(|t| t.get(english).copied()).unwrap_or(english)
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

    #[test]
    fn falls_back_and_formats() {
        set_language(Lang::English);
        assert_eq!(t!("Sign in"), "Sign in");
        assert_eq!(t!("Playing {game}", game = "Dota 2"), "Playing Dota 2");
        set_language(Lang::Russian);
        assert_eq!(t!("Sign in"), "Войти");
        assert_eq!(t!("Playing {game}", game = "Dota 2"), "Играет в Dota 2");
        set_language(Lang::English);
    }

    /// Every `t!("...")` in the app must have a Russian translation.
    #[test]
    fn russian_covers_all_strings() {
        let ru = table(Lang::Russian).unwrap();
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
                    if !ru.contains_key(key) && !missing.contains(&key.to_string()) {
                        missing.push(key.to_string());
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing Russian strings:\n{}", missing.iter().map(|m| format!("\"{m}\" = \"\"")).collect::<Vec<_>>().join("\n"));
    }
}
