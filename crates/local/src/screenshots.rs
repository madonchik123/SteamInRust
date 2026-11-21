//! The player's screenshots on this PC, where Steam keeps them:
//! `userdata/<account>/760/remote/<game>/screenshots/<name>.jpg`, thumbnails
//! in `thumbnails/` beside them, and the details (caption, who can see it,
//! whether it's uploaded) in `userdata/<account>/760/screenshots.vdf`.
//!
//! Files on disk that the list doesn't know yet (screenshots this app took)
//! are shown too, and get an entry the first time they're edited or uploaded.
//! `screenshots.vdf` is only ever replaced whole, after a successful parse,
//! keeping every entry and key this code doesn't understand.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use vdf::{Obj, Value};

/// Who can see an uploaded screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Visibility {
    #[default]
    Public,
    Friends,
    Private,
    Unlisted,
}

impl Visibility {
    pub const ALL: [Visibility; 4] = [Visibility::Public, Visibility::Friends, Visibility::Private, Visibility::Unlisted];

    /// Steam's EUCMFilePrivacyState: `Permissions` in `screenshots.vdf` and
    /// the upload message.
    pub fn ucm(self) -> u32 {
        match self {
            Visibility::Private => 2,
            Visibility::Friends => 4,
            Visibility::Public => 8,
            Visibility::Unlisted => 16,
        }
    }

    pub fn from_ucm(v: u64) -> Self {
        match v {
            2 => Visibility::Private,
            4 => Visibility::Friends,
            16 => Visibility::Unlisted,
            _ => Visibility::Public,
        }
    }

    /// Steam's EPublishedFileVisibility: what the published-file calls use.
    pub fn published(self) -> u32 {
        match self {
            Visibility::Public => 0,
            Visibility::Friends => 1,
            Visibility::Private => 2,
            Visibility::Unlisted => 3,
        }
    }

    pub fn from_published(v: i64) -> Self {
        match v {
            1 => Visibility::Friends,
            2 => Visibility::Private,
            3 => Visibility::Unlisted,
            _ => Visibility::Public,
        }
    }
}

/// One screenshot on this PC.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    /// The game's folder under `remote` (a Steam app id, or a non-Steam game's id).
    pub game: u64,
    /// File name, like `20261108142500_1.jpg`.
    pub name: String,
    pub path: PathBuf,
    pub thumbnail: Option<PathBuf>,
    /// Seconds since 1970.
    pub created: u64,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub caption: String,
    pub spoiler: bool,
    pub visibility: Visibility,
    /// Its id on Steam, once uploaded.
    pub published: Option<u64>,
    /// Steam's handle for deleting it, once uploaded.
    pub handle: Option<u64>,
}

/// What can be changed about a screenshot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Edit {
    pub caption: String,
    pub spoiler: bool,
    pub visibility: Visibility,
}

/// Steam's "no handle" value.
const NO_HANDLE: u64 = u64::MAX;

fn base(root: &Path, account: u32) -> PathBuf {
    root.join("userdata").join(account.to_string()).join("760")
}

fn list_file(root: &Path, account: u32) -> PathBuf {
    base(root, account).join("screenshots.vdf")
}

/// The folder a game's screenshots are in.
pub fn game_folder(root: &Path, account: u32, game: u64) -> PathBuf {
    base(root, account).join("remote").join(game.to_string()).join("screenshots")
}

fn is_picture(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png")
}

/// A plain file name: no folders, and a picture.
fn safe_name(name: &str) -> bool {
    crate::is_valid_folder_name(name) && is_picture(name)
}

fn last_part(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The root object of `screenshots.vdf`, whatever its capitalisation.
fn root_key(doc: &Obj) -> Option<usize> {
    doc.0.iter().position(|(k, v)| k.eq_ignore_ascii_case("screenshots") && matches!(v, Value::Obj(_)))
}

/// Entries of the list, by (game, file name).
fn entries(doc: &Obj) -> HashMap<(u64, String), &Obj> {
    let mut out = HashMap::new();
    let Some(i) = root_key(doc) else { return out };
    let Value::Obj(root) = &doc.0[i].1 else { return out };
    for (game, v) in root.iter() {
        let (Ok(game), Some(list)) = (game.parse::<u64>(), v.as_obj()) else { continue };
        for (_, e) in list.iter() {
            let Some(e) = e.as_obj() else { continue };
            let Some(file) = e.str("filename") else { continue };
            out.insert((game, last_part(file).to_string()), e);
        }
    }
    out
}

fn read_list(path: &Path) -> Result<Obj, String> {
    match fs::read_to_string(path) {
        Ok(text) => vdf::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Obj::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Replace the list in one step, keeping a copy of Steam's original the
/// first time this app changes it.
fn write_list(path: &Path, doc: &Obj) -> Result<(), String> {
    if path.is_file() {
        let backup = path.with_file_name("screenshots.vdf.sir-backup");
        if !backup.exists() {
            fs::copy(path, &backup).map_err(|e| format!("{}: {e}", backup.display()))?;
        }
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let tmp = path.with_file_name("screenshots.vdf.sir-tmp");
    let written = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(vdf::write(doc).as_bytes())?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    written.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

fn modified_secs(m: &fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Every screenshot on this PC for this account, newest first.
pub fn list(root: &Path, account: u32) -> Vec<Shot> {
    let doc = read_list(&list_file(root, account)).unwrap_or_default();
    let known = entries(&doc);
    let mut out = Vec::new();
    let Ok(games) = fs::read_dir(base(root, account).join("remote")) else { return out };
    for g in games.flatten() {
        let Some(game) = g.file_name().to_str().and_then(|n| n.parse::<u64>().ok()) else { continue };
        let dir = g.path().join("screenshots");
        let Ok(files) = fs::read_dir(&dir) else { continue };
        for f in files.flatten() {
            let Some(name) = f.file_name().to_str().map(String::from) else { continue };
            if !safe_name(&name) {
                continue;
            }
            let path = f.path();
            let Ok(m) = fs::symlink_metadata(&path) else { continue };
            if !m.is_file() || crate::is_symlink_or_reparse(&m) {
                continue;
            }
            let thumb = dir.join("thumbnails").join(&name);
            let e = known.get(&(game, name.clone()));
            let num = |k: &str| e.and_then(|e| e.u64(k));
            out.push(Shot {
                game,
                path,
                thumbnail: thumb.is_file().then_some(thumb),
                created: num("creation").filter(|t| *t > 0).unwrap_or_else(|| modified_secs(&m)),
                width: num("width").unwrap_or(0) as u32,
                height: num("height").unwrap_or(0) as u32,
                bytes: m.len(),
                caption: e.and_then(|e| e.str("caption")).unwrap_or_default().to_string(),
                spoiler: num("spoiler").is_some_and(|v| v != 0),
                visibility: num("Permissions").map(Visibility::from_ucm).unwrap_or_default(),
                published: num("publishedfileid").filter(|v| *v != 0),
                handle: num("hscreenshot").filter(|v| *v != 0 && *v != NO_HANDLE),
                name,
            });
        }
    }
    out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| b.name.cmp(&a.name)));
    out
}

/// Set `key` in an entry, replacing its value or adding it.
fn set(e: &mut Obj, key: &str, value: String) {
    match e.0.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
        Some((_, v)) => *v = Value::Str(value),
        None => e.0.push((key.to_string(), Value::Str(value))),
    }
}

/// The list entry for this shot, created if it has none yet.
fn entry_mut<'a>(doc: &'a mut Obj, shot: &Shot) -> Option<&'a mut Obj> {
    let i = match root_key(doc) {
        Some(i) => i,
        None => {
            doc.0.push(("Screenshots".to_string(), Value::Obj(Obj::default())));
            doc.0.len() - 1
        }
    };
    let Value::Obj(root) = &mut doc.0[i].1 else { return None };
    let game = shot.game.to_string();
    let g = match root.0.iter().position(|(k, v)| *k == game && matches!(v, Value::Obj(_))) {
        Some(g) => g,
        None => {
            root.0.push((game.clone(), Value::Obj(Obj::default())));
            root.0.len() - 1
        }
    };
    let Value::Obj(list) = &mut root.0[g].1 else { return None };
    let found = list.0.iter().position(|(_, v)| v.as_obj().and_then(|o| o.str("filename")).is_some_and(|f| last_part(f) == shot.name));
    let at = match found {
        Some(at) => at,
        None => {
            // The next free number: entries are keyed 0, 1, 2…
            let next = list.0.iter().filter_map(|(k, _)| k.parse::<u64>().ok()).max().map_or(0, |n| n + 1);
            let e = Obj::default()
                .with("type", "1")
                .with("filename", format!("{game}/screenshots/{}", shot.name))
                .with("thumbnail", format!("{game}/screenshots/thumbnails/{}", shot.name))
                .with("vrfilename", "")
                .with("imported", "0")
                .with("location", "")
                .with("width", shot.width.to_string())
                .with("height", shot.height.to_string())
                .with("gameid", game.as_str())
                .with("creation", shot.created.to_string())
                .with("caption", "")
                .with("Permissions", Visibility::default().ucm().to_string())
                .with("hscreenshot", NO_HANDLE.to_string());
            list.0.push((next.to_string(), Value::Obj(e)));
            list.0.len() - 1
        }
    };
    match &mut list.0[at].1 {
        Value::Obj(e) => Some(e),
        Value::Str(_) => None,
    }
}

/// Only ever touch a shot inside its own game's folder.
fn check(root: &Path, account: u32, shot: &Shot) -> Result<(), String> {
    if !safe_name(&shot.name) || shot.path != game_folder(root, account, shot.game).join(&shot.name) {
        return Err("That screenshot isn't in your screenshot folder.".into());
    }
    Ok(())
}

/// Save a screenshot's caption, spoiler tag and visibility on this PC.
pub fn save_edit(root: &Path, account: u32, shot: &Shot, edit: &Edit) -> Result<(), String> {
    check(root, account, shot)?;
    let path = list_file(root, account);
    let mut doc = read_list(&path)?;
    let e = entry_mut(&mut doc, shot).ok_or("Steam's screenshot list has an unexpected layout.")?;
    // Captions are one line in Steam's list.
    set(e, "caption", edit.caption.replace(['\r', '\n'], " "));
    set(e, "spoiler", u8::from(edit.spoiler).to_string());
    set(e, "Permissions", edit.visibility.ucm().to_string());
    write_list(&path, &doc)
}

/// Record that a screenshot was uploaded (or, with `None`, that it no
/// longer is).
pub fn set_uploaded(root: &Path, account: u32, shot: &Shot, ids: Option<(u64, u64)>) -> Result<(), String> {
    check(root, account, shot)?;
    let path = list_file(root, account);
    let mut doc = read_list(&path)?;
    let e = entry_mut(&mut doc, shot).ok_or("Steam's screenshot list has an unexpected layout.")?;
    let (published, handle) = ids.unwrap_or((0, NO_HANDLE));
    set(e, "publishedfileid", published.to_string());
    set(e, "hscreenshot", handle.to_string());
    write_list(&path, &doc)
}

/// Delete a screenshot from this PC: the picture, its thumbnail and its
/// entry. An uploaded copy stays on Steam.
pub fn delete_local(root: &Path, account: u32, shot: &Shot) -> Result<(), String> {
    check(root, account, shot)?;
    let m = fs::symlink_metadata(&shot.path).map_err(|e| format!("{}: {e}", shot.path.display()))?;
    if !m.is_file() || crate::is_symlink_or_reparse(&m) {
        return Err("That screenshot isn't in your screenshot folder.".into());
    }
    // The list first: if it can't be read, nothing has been deleted yet.
    let path = list_file(root, account);
    let mut doc = read_list(&path)?;
    let mut changed = false;
    if let Some(i) = root_key(&doc)
        && let Value::Obj(r) = &mut doc.0[i].1
        && let Some((_, Value::Obj(list))) = r.0.iter_mut().find(|(k, _)| *k == shot.game.to_string())
    {
        let before = list.0.len();
        list.0.retain(|(_, v)| !v.as_obj().and_then(|o| o.str("filename")).is_some_and(|f| last_part(f) == shot.name));
        changed = list.0.len() != before;
    }
    fs::remove_file(&shot.path).map_err(|e| format!("{}: {e}", shot.path.display()))?;
    let thumb = game_folder(root, account, shot.game).join("thumbnails").join(&shot.name);
    let _ = fs::remove_file(thumb);
    if changed {
        write_list(&path, &doc)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);
    const ME: u32 = 1234;

    struct Fake(PathBuf);

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const LIST: &str = "\"Screenshots\"\n{\n\t\"shortcutnames\"\n\t{\n\t\t\"99\"\t\t\"My Game\"\n\t}\n\t\"440\"\n\t{\n\t\t\"0\"\n\t\t{\n\t\t\t\"type\"\t\t\"1\"\n\t\t\t\"filename\"\t\t\"440/screenshots/20240101120000_1.jpg\"\n\t\t\t\"thumbnail\"\t\t\"440/screenshots/thumbnails/20240101120000_1.jpg\"\n\t\t\t\"width\"\t\t\"1920\"\n\t\t\t\"height\"\t\t\"1080\"\n\t\t\t\"creation\"\t\t\"1704110400\"\n\t\t\t\"caption\"\t\t\"Old caption\"\n\t\t\t\"Permissions\"\t\t\"4\"\n\t\t\t\"hscreenshot\"\t\t\"777\"\n\t\t\t\"publishedfileid\"\t\t\"888\"\n\t\t\t\"somethingnew\"\t\t\"keep me\"\n\t\t}\n\t}\n}\n";

    fn fake() -> Fake {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("sir-shots-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = game_folder(&root, ME, 440);
        fs::create_dir_all(dir.join("thumbnails")).unwrap();
        fs::write(dir.join("20240101120000_1.jpg"), b"known").unwrap();
        fs::write(dir.join("thumbnails").join("20240101120000_1.jpg"), b"t").unwrap();
        fs::write(dir.join("20250101120000_1.jpg"), b"not in the list").unwrap();
        fs::write(dir.join("notes.txt"), b"not a picture").unwrap();
        fs::write(list_file(&root, ME), LIST).unwrap();
        Fake(root)
    }

    fn shot(f: &Fake, name: &str) -> Shot {
        list(&f.0, ME).into_iter().find(|s| s.name == name).unwrap()
    }

    #[test]
    fn lists_known_and_new_screenshots() {
        let f = fake();
        let all = list(&f.0, ME);
        assert_eq!(all.len(), 2, "pictures only");
        let k = shot(&f, "20240101120000_1.jpg");
        assert_eq!((k.game, k.width, k.height, k.created), (440, 1920, 1080, 1704110400));
        assert_eq!(k.caption, "Old caption");
        assert_eq!(k.visibility, Visibility::Friends);
        assert_eq!((k.published, k.handle), (Some(888), Some(777)));
        assert!(k.thumbnail.is_some());
        let new = shot(&f, "20250101120000_1.jpg");
        assert_eq!(new.published, None);
        assert!(new.thumbnail.is_none());
    }

    #[test]
    fn editing_keeps_what_it_doesnt_know_and_backs_up() {
        let f = fake();
        let k = shot(&f, "20240101120000_1.jpg");
        save_edit(&f.0, ME, &k, &Edit { caption: "New \"quoted\"\ncaption".into(), spoiler: true, visibility: Visibility::Private }).unwrap();
        let k = shot(&f, "20240101120000_1.jpg");
        assert_eq!(k.caption, "New \"quoted\" caption");
        assert!(k.spoiler);
        assert_eq!(k.visibility, Visibility::Private);
        assert_eq!(k.published, Some(888));
        let text = fs::read_to_string(list_file(&f.0, ME)).unwrap();
        assert!(text.contains("somethingnew") && text.contains("shortcutnames"));
        let backup = list_file(&f.0, ME).with_file_name("screenshots.vdf.sir-backup");
        assert_eq!(fs::read_to_string(backup).unwrap(), LIST);
    }

    #[test]
    fn a_new_screenshot_gets_an_entry_when_edited() {
        let f = fake();
        let n = shot(&f, "20250101120000_1.jpg");
        save_edit(&f.0, ME, &n, &Edit { caption: "Hi".into(), ..Default::default() }).unwrap();
        assert_eq!(shot(&f, "20250101120000_1.jpg").caption, "Hi");
        let text = fs::read_to_string(list_file(&f.0, ME)).unwrap();
        assert!(text.contains("440/screenshots/20250101120000_1.jpg"));
        assert_eq!(shot(&f, "20240101120000_1.jpg").caption, "Old caption");
    }

    #[test]
    fn works_without_a_list_file() {
        let f = fake();
        fs::remove_file(list_file(&f.0, ME)).unwrap();
        let n = shot(&f, "20240101120000_1.jpg");
        save_edit(&f.0, ME, &n, &Edit { caption: "First".into(), ..Default::default() }).unwrap();
        assert_eq!(shot(&f, "20240101120000_1.jpg").caption, "First");
    }

    #[test]
    fn never_overwrites_a_list_it_cant_read() {
        let f = fake();
        let k = shot(&f, "20240101120000_1.jpg");
        fs::write(list_file(&f.0, ME), "\"Screenshots\" { \"broken").unwrap();
        assert!(save_edit(&f.0, ME, &k, &Edit::default()).is_err());
        assert_eq!(fs::read_to_string(list_file(&f.0, ME)).unwrap(), "\"Screenshots\" { \"broken");
        assert!(delete_local(&f.0, ME, &k).is_err());
        assert!(k.path.exists(), "nothing deleted when the list can't be updated");
    }

    #[test]
    fn records_and_clears_an_upload() {
        let f = fake();
        let n = shot(&f, "20250101120000_1.jpg");
        set_uploaded(&f.0, ME, &n, Some((55, 66))).unwrap();
        let n = shot(&f, "20250101120000_1.jpg");
        assert_eq!((n.published, n.handle), (Some(55), Some(66)));
        set_uploaded(&f.0, ME, &n, None).unwrap();
        let n = shot(&f, "20250101120000_1.jpg");
        assert_eq!((n.published, n.handle), (None, None));
    }

    #[test]
    fn deletes_picture_thumbnail_and_entry_only() {
        let f = fake();
        let k = shot(&f, "20240101120000_1.jpg");
        let thumb = k.thumbnail.clone().unwrap();
        delete_local(&f.0, ME, &k).unwrap();
        assert!(!k.path.exists() && !thumb.exists());
        assert_eq!(list(&f.0, ME).len(), 1);
        let text = fs::read_to_string(list_file(&f.0, ME)).unwrap();
        assert!(!text.contains("20240101120000_1") && text.contains("shortcutnames"));
    }

    #[test]
    fn refuses_paths_outside_the_folder() {
        let f = fake();
        let outside = f.0.join("precious.jpg");
        fs::write(&outside, b"keep").unwrap();
        let mut bad = shot(&f, "20240101120000_1.jpg");
        bad.path = outside.clone();
        assert!(delete_local(&f.0, ME, &bad).is_err());
        assert!(outside.exists());
        let mut bad = shot(&f, "20240101120000_1.jpg");
        bad.name = "../../precious.jpg".into();
        assert!(delete_local(&f.0, ME, &bad).is_err());
        assert!(save_edit(&f.0, ME, &bad, &Edit::default()).is_err());
    }

    #[test]
    fn the_two_visibility_numberings_are_kept_apart() {
        for v in Visibility::ALL {
            assert_eq!(Visibility::from_ucm(v.ucm() as u64), v);
            assert_eq!(Visibility::from_published(v.published() as i64), v);
        }
        assert_eq!(Visibility::Private.ucm(), 2);
        assert_eq!(Visibility::Friends.published(), 1);
    }
}
