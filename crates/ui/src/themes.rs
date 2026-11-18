//! Theme packages: the built-in themes, loading a theme from a folder, and
//! the user's choice with their own accent color and roundness on top.
//!
//! A package is a folder with a `theme.toml` and the files it names (fonts,
//! pictures). Built-in themes are the same files, built into the app.
//! A theme file only needs what it changes: everything else comes from the
//! theme it `extends`, or from the default theme. A theme that can't be read
//! falls back to the default theme with a warning in the log; the app never
//! stops over a theme.

use crate::theme::{Assets, Fill, Theme};
use egui::Color32;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// The theme new users get.
pub const DEFAULT_ID: &str = "steaminrust";

/// A built-in package: its id and files (`theme.toml` first).
struct Builtin {
    id: &'static str,
    files: &'static [(&'static str, &'static [u8])],
}

const BUILTINS: &[Builtin] = &[
    Builtin { id: "steaminrust", files: &[("theme.toml", include_bytes!("../themes/steaminrust/theme.toml"))] },
    Builtin { id: "classic", files: &[("theme.toml", include_bytes!("../themes/classic/theme.toml"))] },
    Builtin { id: "aurora", files: &[("theme.toml", include_bytes!("../themes/aurora/theme.toml"))] },
    Builtin { id: "blue", files: &[("theme.toml", include_bytes!("../themes/blue/theme.toml"))] },
    Builtin { id: "oled", files: &[("theme.toml", include_bytes!("../themes/oled/theme.toml"))] },
    Builtin { id: "daylight", files: &[("theme.toml", include_bytes!("../themes/daylight/theme.toml"))] },
];

/// The largest `theme.toml` and the largest other file a package may have.
const MAX_MANIFEST: u64 = 1 << 20;
const MAX_FILE: u64 = 32 << 20;

fn builtin_text(id: &str) -> Option<&'static str> {
    let b = BUILTINS.iter().find(|b| b.id == id)?;
    std::str::from_utf8(b.files.first()?.1).ok()
}

/// A built-in theme's table with everything it extends filled in.
fn resolved_table(id: &str, depth: usize) -> Result<toml::Table, String> {
    let text = builtin_text(id).ok_or_else(|| format!("no built-in theme {id:?}"))?;
    let own: toml::Table = toml::from_str(text).map_err(|e| format!("built-in theme {id}: {e}"))?;
    if id == DEFAULT_ID {
        return Ok(own);
    }
    let parent = own.get("extends").and_then(|v| v.as_str()).unwrap_or(DEFAULT_ID).to_string();
    if depth > 8 || parent == id {
        return Err(format!("theme {id:?} extends itself"));
    }
    let mut base = resolved_table(&parent, depth + 1)?;
    merge(&mut base, own);
    Ok(base)
}

/// Lay `top` over `base`. Tables merge key by key, except fills (a table
/// with `linear`, `radial` or `image`), which replace whole.
fn merge(base: &mut toml::Table, top: toml::Table) {
    let atomic = |t: &toml::Table| ["linear", "radial", "image"].iter().any(|k| t.contains_key(*k));
    for (k, v) in top {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(t)) if !atomic(b) && !atomic(&t) => merge(b, t),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

fn builtin_assets(id: &str) -> Assets {
    let Some(b) = BUILTINS.iter().find(|b| b.id == id) else { return Assets::default() };
    let map: HashMap<String, Arc<[u8]>> = b.files.iter().skip(1).map(|(p, d)| (p.to_string(), Arc::from(*d))).collect();
    Assets(Some(Arc::new(map)))
}

/// The default theme ("SteamInRust").
pub fn default_theme() -> Theme {
    static DEFAULT: OnceLock<Theme> = OnceLock::new();
    DEFAULT.get_or_init(|| builtin(DEFAULT_ID).expect("the default theme is built in and valid")).clone()
}

/// A built-in theme by id.
pub fn builtin(id: &str) -> Option<Theme> {
    let table = match resolved_table(id, 0) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("{e}");
            return None;
        }
    };
    match toml::Value::Table(table).try_into::<Theme>() {
        Ok(mut t) => {
            t.assets = builtin_assets(id);
            Some(t)
        }
        Err(e) => {
            tracing::warn!("built-in theme {id}: {e}");
            None
        }
    }
}

/// Every built-in theme, the default first.
pub fn builtins() -> Vec<Theme> {
    BUILTINS.iter().filter_map(|b| builtin(b.id)).collect()
}

/// Read a theme file's text. What it leaves out comes from the built-in
/// theme it `extends`, or from the default theme. Doesn't load any files
/// it names (see [`load_dir`]).
pub fn parse(text: &str) -> Result<Theme, String> {
    let own: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let parent = own.get("extends").and_then(|v| v.as_str()).unwrap_or(DEFAULT_ID).to_string();
    for key in ["id", "name"] {
        if !own.get(key).is_some_and(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())) {
            return Err(format!("a theme needs its own `{key}`"));
        }
    }
    let mut base = resolved_table(&parent, 0)?;
    // The preview picture belongs to the theme it came from.
    base.remove("preview");
    merge(&mut base, own);
    let theme: Theme = toml::Value::Table(base).try_into().map_err(|e: toml::de::Error| e.to_string())?;
    if theme.format > crate::theme::FORMAT {
        tracing::warn!("theme {:?} is for a newer version of the app (format {}); parts of it may not show", theme.id, theme.format);
    }
    Ok(theme)
}

/// Write a theme as its `theme.toml` text.
pub fn to_toml(t: &Theme) -> String {
    toml::to_string_pretty(t).unwrap_or_default()
}

/// A path inside a package: relative, no `..`, no drive or web address.
fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !p.contains("://")
        && !p.contains(':')
        && !p.starts_with('/')
        && !p.starts_with('\\')
        && path.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Every file a theme names.
fn referenced_files(t: &Theme) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    out.extend(t.preview.clone());
    for f in [&t.fonts.regular, &t.fonts.medium, &t.fonts.semibold, &t.fonts.bold, &t.images.logo, &t.icons.font] {
        out.extend(f.clone());
    }
    let mut fill = |f: &Fill| {
        if let Fill::Image { image, .. } = f {
            out.push(image.clone());
        }
    };
    for f in [&t.backgrounds.page, &t.backgrounds.store, &t.backgrounds.library, &t.backgrounds.community, &t.backgrounds.profile] {
        fill(f);
    }
    for (_, c) in t.components.all() {
        for st in [&c.normal, &c.hover, &c.pressed, &c.selected, &c.disabled] {
            if let Some(f) = &st.fill {
                fill(f);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Load a theme package from a folder: its `theme.toml` and the files it
/// names, which must be inside the folder (no `..`, no absolute paths, no
/// web addresses). A file that's missing or not allowed is left out with a
/// warning, and whatever uses it falls back.
pub fn load_dir(dir: &Path) -> Result<Theme, String> {
    let root = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let manifest = root.join("theme.toml");
    let size = std::fs::metadata(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?.len();
    if size > MAX_MANIFEST {
        return Err(format!("{} is too large", manifest.display()));
    }
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut theme = parse(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let mut files: HashMap<String, Arc<[u8]>> = HashMap::new();
    for rel in referenced_files(&theme) {
        match read_inside(&root, &rel) {
            Ok(bytes) => {
                files.insert(rel, bytes);
            }
            Err(e) => tracing::warn!("theme {:?}: {rel}: {e}", theme.id),
        }
    }
    theme.assets = Assets(Some(Arc::new(files)));
    for p in theme.problems() {
        tracing::warn!("theme {:?}: {p}", theme.id);
    }
    Ok(theme)
}

fn read_inside(root: &Path, rel: &str) -> Result<Arc<[u8]>, String> {
    if !safe_relative(rel) {
        return Err("only files inside the theme's folder can be used".into());
    }
    let path: PathBuf = root.join(rel).canonicalize().map_err(|e| e.to_string())?;
    if !path.starts_with(root) {
        return Err("only files inside the theme's folder can be used".into());
    }
    let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE {
        return Err("file too large".into());
    }
    std::fs::read(&path).map(Arc::from).map_err(|e| e.to_string())
}

/// [`load_dir`], or the default theme (with a warning) if it can't be read.
pub fn load_dir_or_default(dir: &Path) -> Theme {
    match load_dir(dir) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("theme not loaded, using the default: {e}");
            default_theme()
        }
    }
}

/// A picture from the theme's package as an image address egui can draw
/// (`None` if the package doesn't have it).
pub fn image_uri(ctx: &egui::Context, t: &Theme, path: &str) -> Option<String> {
    let bytes = t.assets.get(path)?;
    let uri = format!("bytes://theme/{}/{}", t.id, path);
    let id = egui::Id::unique(("theme-image", uri.as_str()));
    if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        ctx.include_bytes(uri.clone(), egui::load::Bytes::Shared(bytes));
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
    Some(uri)
}

// ---------- the user's choice ----------

/// Which theme the user picked, with their own changes on top (saved in
/// the app's settings; the theme itself isn't).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeChoice {
    /// A built-in theme's id.
    pub id: String,
    #[serde(with = "opt_hex")]
    pub accent: Option<Color32>,
    #[serde(with = "opt_hex")]
    pub accent_2: Option<Color32>,
    /// The large corner radius; the small one follows in proportion.
    pub radius: Option<u8>,
}

/// The theme for a choice: the built-in theme (the default if it's
/// unknown) with the user's changes on top.
pub fn resolve(choice: &ThemeChoice) -> Theme {
    let mut t = builtin(&choice.id).unwrap_or_else(|| {
        if !choice.id.is_empty() {
            tracing::warn!("theme {:?} isn't built in; using the default", choice.id);
        }
        default_theme()
    });
    apply_overrides(&mut t, choice);
    t
}

/// Put the user's accent colors and roundness on a theme.
pub fn apply_overrides(t: &mut Theme, choice: &ThemeChoice) {
    if let Some(c) = choice.accent {
        t.tokens.accent = c;
    }
    if let Some(c) = choice.accent_2 {
        t.tokens.accent_2 = c;
    }
    if let Some(r) = choice.radius {
        let (large, small) = (t.tokens.radius.max(1) as f32, t.tokens.radius_small as f32);
        t.tokens.radius_small = (small * r as f32 / large).round() as u8;
        t.tokens.radius = r;
    }
}

/// The choice that gives this theme: its id, and whatever differs from the
/// built-in theme as the user's changes.
pub fn choice_for(t: &Theme) -> ThemeChoice {
    let base = builtin(&t.id);
    let differs = |a: Color32, b: Option<Color32>| (Some(a) != b).then_some(a);
    ThemeChoice {
        id: t.id.clone(),
        accent: differs(t.accent, base.as_ref().map(|b| b.accent)),
        accent_2: differs(t.accent_2, base.as_ref().map(|b| b.accent_2)),
        radius: (Some(t.radius) != base.as_ref().map(|b| b.radius)).then_some(t.radius),
    }
}

/// A theme saved by an app version before theme packages: which built-in
/// theme it was, and the user's accent and roundness. Read field by field
/// exactly as those versions wrote them (egui's color format, nothing
/// optional), so old settings never fail to load.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LegacyTheme {
    pub name: String,
    pub accent: Color32,
    pub accent_2: Color32,
    pub radius: u8,
}

impl LegacyTheme {
    /// Whether this came from saved settings (the default means none).
    pub fn is_set(&self) -> bool {
        !self.name.is_empty()
    }

    /// The theme package that matches, keeping the user's changes.
    pub fn to_choice(&self) -> ThemeChoice {
        let id = match self.name.as_str() {
            "Midnight" => "classic",
            "Aurora" => "aurora",
            "Classic" => "blue",
            "OLED" => "oled",
            "Daylight" => "daylight",
            _ => DEFAULT_ID,
        };
        let mut t = resolve(&ThemeChoice { id: id.into(), ..Default::default() });
        if self.accent != Color32::TRANSPARENT {
            t.tokens.accent = self.accent;
        }
        if self.accent_2 != Color32::TRANSPARENT {
            t.tokens.accent_2 = self.accent_2;
        }
        if self.radius > 0 {
            t.tokens.radius = self.radius;
        }
        choice_for(&t)
    }
}

/// `Option<Color32>` as `"#RRGGBB"` (also reads egui's `[r, g, b, a]`).
mod opt_hex {
    use egui::Color32;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(c: &Option<Color32>, s: S) -> Result<S::Ok, S::Error> {
        match c {
            Some(c) => s.serialize_some(&crate::theme::to_hex(*c)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Color32>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(s) => crate::theme::parse_hex(&s).map(Some).ok_or_else(|| serde::de::Error::custom(format!("not a color: {s:?}"))),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same_look(a: &Theme, b: &Theme) -> bool {
        let mut a = a.clone();
        a.assets = b.assets.clone();
        &a == b
    }

    #[test]
    fn every_builtin_loads_cleanly() {
        let all = builtins();
        assert_eq!(all.len(), BUILTINS.len(), "a built-in theme didn't load");
        for t in &all {
            assert_eq!(t.format, crate::theme::FORMAT);
            assert!(t.problems().is_empty(), "{}: {:?}", t.id, t.problems());
        }
        assert_eq!(all[0].id, DEFAULT_ID);
        assert_eq!(Theme::default().id, DEFAULT_ID);
    }

    #[test]
    fn every_builtin_round_trips_through_its_file() {
        for t in builtins() {
            let text = to_toml(&t);
            let back = parse(&text).unwrap_or_else(|e| panic!("{}: {e}\n{text}", t.id));
            assert!(same_look(&back, &t), "{} changed going through its file", t.id);
        }
    }

    #[test]
    fn a_partial_theme_falls_back_to_the_default() {
        let t = parse("format = 1\nid = \"mine\"\nname = \"Mine\"\n[tokens]\naccent = \"#FF0000\"\n[components.button_primary.hover]\nfill = \"#00FF00\"\n").unwrap();
        let d = default_theme();
        assert_eq!(t.accent, Color32::from_rgb(255, 0, 0));
        assert_eq!(t.bg, d.bg);
        assert_eq!(t.components.button_primary.normal, d.components.button_primary.normal);
        assert_eq!(t.components.button_primary.hover.fill, Some(Fill::Color("#00FF00".into())));
        assert_eq!(t.components.button_primary.hover.text, d.components.button_primary.hover.text);
        assert_eq!(t.layout, d.layout);
    }

    #[test]
    fn a_theme_can_extend_a_builtin() {
        let t = parse("format = 1\nid = \"mine\"\nname = \"Mine\"\nextends = \"classic\"\n[tokens]\nbg = \"#000000\"\n").unwrap();
        let c = builtin("classic").unwrap();
        assert_eq!(t.bg, Color32::BLACK);
        assert_eq!(t.accent, c.accent);
        assert_eq!(t.layout, c.layout);
    }

    #[test]
    fn broken_themes_are_refused_not_fatal() {
        assert!(parse("this is not toml [").is_err());
        assert!(parse("format = 1\nname = \"No id\"\n").is_err());
        assert!(parse("format = 1\nid = \"x\"\nname = \"X\"\n[tokens]\nbg = 5\n").is_err());
        assert!(parse("format = 1\nid = \"x\"\nname = \"X\"\nextends = \"nope\"\n").is_err());
        let missing = std::env::temp_dir().join("sir-no-such-theme-folder");
        assert_eq!(load_dir_or_default(&missing).id, DEFAULT_ID);
        // Unknown color names load, and are reported.
        let t = parse("format = 1\nid = \"x\"\nname = \"X\"\n[components.card.normal]\nfill = \"blurple\"\n").unwrap();
        assert!(t.problems().iter().any(|p| p.contains("blurple")));
    }

    #[test]
    fn colors_can_be_worked_out_from_tokens() {
        let t = default_theme();
        assert_eq!(t.color(&"accent".into()), t.accent);
        assert_eq!(t.color(&"#102030".into()), Color32::from_rgb(0x10, 0x20, 0x30));
        assert_eq!(t.color(&"accent@0.5".into()), t.accent.gamma_multiply(0.5));
        assert_eq!(t.color(&"mix(black, white, 0)".into()), Color32::BLACK);
        assert_eq!(t.color(&"text_on(play)".into()), crate::theme::text_on(t.play));
        assert_eq!(t.color(&"lighten(darken(accent, 0.5), 0)".into()), crate::widgets::mix(t.accent, Color32::BLACK, 0.5));
    }

    #[test]
    fn a_folder_theme_only_reads_files_inside_it() {
        let dir = std::env::temp_dir().join(format!("sir-theme-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images/bg.png"), b"not really a png").unwrap();
        std::fs::write(dir.parent().unwrap().join("sir-outside.txt"), b"secret").unwrap();
        std::fs::write(
            dir.join("theme.toml"),
            "format = 1\nid = \"t\"\nname = \"T\"\n[images]\nlogo = \"../sir-outside.txt\"\n[fonts]\nregular = \"C:/Windows/Fonts/arial.ttf\"\n[backgrounds]\nstore = { image = \"images/bg.png\" }\nlibrary = { image = \"https://example.com/x.png\" }\n",
        )
        .unwrap();
        let t = load_dir(&dir).unwrap();
        assert!(t.assets.get("images/bg.png").is_some());
        assert!(t.assets.get("../sir-outside.txt").is_none());
        assert!(t.assets.get("C:/Windows/Fonts/arial.ttf").is_none());
        assert!(t.assets.get("https://example.com/x.png").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_changes_sit_on_top_of_the_theme() {
        let mut t = builtin("classic").unwrap();
        t.tokens.accent = Color32::from_rgb(1, 2, 3);
        t.tokens.radius = 6;
        let choice = choice_for(&t);
        assert_eq!(choice.id, "classic");
        assert_eq!(choice.accent, Some(Color32::from_rgb(1, 2, 3)));
        assert_eq!(choice.accent_2, None);
        let back = resolve(&choice);
        assert_eq!(back.accent, Color32::from_rgb(1, 2, 3));
        assert_eq!(back.radius, 6);
        assert_eq!(back.radius_small, 4);
        // Saved settings from before theme packages.
        let old = LegacyTheme { name: "Aurora".into(), accent: builtin("aurora").unwrap().accent, accent_2: Color32::TRANSPARENT, radius: 0 };
        assert_eq!(old.to_choice(), ThemeChoice { id: "aurora".into(), ..Default::default() });
    }
}

#[cfg(test)]
mod example_tests {
    /// The example theme in the docs loads, with every file it names and no problems.
    #[test]
    fn the_documented_example_theme_loads() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/examples/minimal-theme");
        let t = super::load_dir(&dir).unwrap();
        assert_eq!(t.id, "ember");
        assert!(t.problems().is_empty(), "{:?}", t.problems());
        assert!(t.assets.get("images/dots.svg").is_some());
    }
}
