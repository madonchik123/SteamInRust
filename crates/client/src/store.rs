//! Saved logins ("remember me").
//!
//! The account list (name, Steam ID, display name, avatar) is a file in the
//! user's config folder, readable only by them. The credentials — the refresh
//! token and Steam Guard data — are never written to a file: they're kept in
//! the OS credential store (Secret Service / GNOME Keyring / KWallet on Linux,
//! Credential Manager on Windows, Keychain on macOS), encrypted and locked
//! with the user's login. Without a credential store, sign-ins aren't
//! remembered at all.

use model::SteamId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedAccount {
    pub account_name: String,
    pub steam_id: SteamId,
    #[serde(default)]
    pub persona_name: String,
    #[serde(default)]
    pub avatar_hash: Option<String>,
    /// From the credential store ([`credentials`]); empty from [`load`] and
    /// [`find`]. Read from old plaintext files only to move it out of them.
    #[serde(default, skip_serializing)]
    pub refresh_token: String,
    /// Like `refresh_token`.
    #[serde(default, skip_serializing)]
    pub guard_data: Option<String>,
}

impl std::fmt::Debug for SavedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedAccount")
            .field("account_name", &self.account_name)
            .field("steam_id", &self.steam_id)
            .field("persona_name", &self.persona_name)
            .field("avatar_hash", &self.avatar_hash)
            .field("refresh_token", &"[REDACTED]")
            .field("guard_data", &self.guard_data.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    // SAFETY: POSIX `getuid()` has no preconditions and never fails.
    unsafe { getuid() }
}

fn fallback_config_dir() -> PathBuf {
    #[cfg(unix)]
    {
        PathBuf::from(format!("/tmp/steaminrust-{}", current_uid()))
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .map(|base| PathBuf::from(base).join("SteamInRust"))
            .unwrap_or_else(|| std::env::temp_dir().join("SteamInRust"))
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::env::temp_dir().join("SteamInRust")
    }
}

fn resolve_config_dir(primary: Option<PathBuf>) -> PathBuf {
    let dir = primary.filter(|p| !p.as_os_str().is_empty() && p != std::path::Path::new(".")).unwrap_or_else(fallback_config_dir);
    let _ = ensure_private_dir_at(&dir, None);
    dir
}

pub fn config_dir() -> PathBuf {
    resolve_config_dir(directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust").map(|d| d.config_dir().to_path_buf()))
}

pub(crate) fn ensure_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let base = directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(fallback_config_dir);
    ensure_private_dir_at(dir, Some(&base))
}

fn ensure_private_dir_at(dir: &std::path::Path, base: Option<&std::path::Path>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true).mode(0o700);
        b.create(dir)?;
        let uid = current_uid();
        let tighten = |p: &std::path::Path| -> std::io::Result<()> {
            let meta = std::fs::symlink_metadata(p)?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "config directory is a symlink or not a directory"));
            }
            if meta.uid() != uid {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "config directory owned by another user"));
            }
            if meta.permissions().mode() & 0o777 != 0o700 {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        };
        if let Some(root) = base.filter(|r| dir.starts_with(r)) {
            let mut cur = root.to_path_buf();
            if cur.exists() {
                tighten(&cur)?;
            }
            if let Ok(rel) = dir.strip_prefix(root) {
                for comp in rel.components() {
                    cur.push(comp);
                    tighten(&cur)?;
                }
            }
        } else {
            tighten(dir)?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = base;
        std::fs::create_dir_all(dir)
    }
}

fn path() -> PathBuf {
    config_dir().join("accounts.json")
}

/// The saved accounts, without credentials (see [`credentials`]).
pub fn load() -> Vec<SavedAccount> {
    let mut list: Vec<SavedAccount> = std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // Files from before the credential store held the tokens in plain text:
    // move them out, then rewrite the file without them.
    if list.iter().any(|a| !a.refresh_token.is_empty() || a.guard_data.is_some()) {
        for a in &mut list {
            if !a.refresh_token.is_empty() || a.guard_data.is_some() {
                if let Err(e) = secrets::save(a) {
                    tracing::warn!("couldn't move the saved sign-in of {} to the credential store, it's forgotten: {e}", a.account_name);
                }
                a.refresh_token.clear();
                a.guard_data = None;
            }
        }
        write(&list);
    }
    list
}

/// A saved account with its refresh token and Steam Guard data from the
/// credential store. The token is empty when the store doesn't have it (or
/// there's no store): the account needs its password again.
pub fn credentials(account: &str) -> Option<SavedAccount> {
    let mut acc = find(account)?;
    match secrets::load(&acc.account_name) {
        Ok((token, guard)) => {
            acc.refresh_token = token.unwrap_or_default();
            acc.guard_data = guard;
        }
        Err(e) => tracing::warn!("couldn't read the saved sign-in of {account}: {e}"),
    }
    Some(acc)
}

/// Write the account list (credentials are skipped by serde), readable
/// only by the user.
fn write(list: &[SavedAccount]) {
    let p = path();
    match serde_json::to_string_pretty(list) {
        Ok(json) => {
            if let Err(e) = write_private(&p, json.as_bytes()) {
                tracing::error!("could not save accounts: {e}");
            }
        }
        Err(e) => tracing::error!("could not encode accounts: {e}"),
    }
}

fn open_unique_temp(dir: &std::path::Path, base_name: &str) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let pid = std::process::id();
    for _ in 0..32 {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u32(pid);
        h.write_u64(seq);
        let tag = h.finish();
        let candidate = dir.join(format!(".{base_name}.{pid}-{seq}-{tag:016x}.tmp"));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&candidate) {
            Ok(f) => return Ok((f, candidate)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "could not create unique temporary file"))
}

fn with_private_temp(target: &std::path::Path, default_name: &str, fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>) -> std::io::Result<()> {
    let dir = target.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "missing parent directory"))?;
    ensure_private_dir(dir)?;
    if let Ok(meta) = std::fs::symlink_metadata(target) {
        if meta.file_type().is_symlink() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "refusing to overwrite symlink"));
        }
    }
    let base_name = target.file_name().and_then(|n| n.to_str()).unwrap_or(default_name);
    let (mut f, tmp) = open_unique_temp(dir, base_name)?;
    let res = (|| -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        fill(&mut f)?;
        f.sync_all()
    })();
    drop(f);
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res?;
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Replace `path` with `bytes`, mode 0600 on Unix inside a 0700 parent dir,
/// using a unique `O_EXCL` temporary file.
pub(crate) fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    with_private_temp(path, "file", |f| f.write_all(bytes))
}

/// Copy `src` to `dst` with mode 0600 on Unix inside 0700 parent directories.
pub(crate) fn copy_private(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let mut reader = std::fs::File::open(src)?;
    with_private_temp(dst, "backup", |f| std::io::copy(&mut reader, f).map(|_| ()))
}

pub fn find(account: &str) -> Option<SavedAccount> {
    load().into_iter().find(|a| a.account_name.eq_ignore_ascii_case(account))
}

/// Insert or update by account name, keeping fields the new value leaves
/// empty. Its credentials go to the credential store (if there's none, the
/// sign-in just isn't remembered).
pub fn upsert(mut acc: SavedAccount) {
    // Load (and so migrate) first: migrating an old plaintext file after
    // saving would overwrite this fresh token with the stale one on disk.
    let mut list = load();
    if !acc.refresh_token.is_empty() {
        if acc.guard_data.is_none() {
            acc.guard_data = secrets::load(&acc.account_name).ok().and_then(|(_, g)| g);
        }
        if let Err(e) = secrets::save(&acc) {
            tracing::warn!("couldn't keep the sign-in of {} in the credential store, it won't be remembered: {e}", acc.account_name);
        }
        acc.refresh_token.clear();
        acc.guard_data = None;
    }
    if let Some(old) = list.iter().find(|a| a.account_name.eq_ignore_ascii_case(&acc.account_name)) {
        if acc.persona_name.is_empty() {
            acc.persona_name = old.persona_name.clone();
        }
        if acc.avatar_hash.is_none() {
            acc.avatar_hash = old.avatar_hash.clone();
        }
    }
    list.retain(|a| !a.account_name.eq_ignore_ascii_case(&acc.account_name));
    list.insert(0, acc);
    write(&list);
}

pub fn update(account: &str, f: impl FnOnce(&mut SavedAccount)) {
    let mut list = load();
    if let Some(a) = list.iter_mut().find(|a| a.account_name.eq_ignore_ascii_case(account)) {
        f(a);
        write(&list);
    }
}

/// Forget an account and its credentials.
pub fn remove(account: &str) {
    // Load (and so migrate) first, so an old plaintext token can't be moved
    // into the store after its entries were deleted.
    let mut list = load();
    if let Err(e) = secrets::delete(account) {
        tracing::warn!("couldn't delete the saved sign-in of {account}: {e}");
    }
    list.retain(|a| !a.account_name.eq_ignore_ascii_case(account));
    write(&list);
}

/// The OS credential store. Each account has two entries (refresh token,
/// Steam Guard data), keyed by the lowercase account name.
mod secrets {
    use super::SavedAccount;

    const SERVICE: &str = "dev.SteamInRust.SteamInRust";

    fn entry(kind: &str, account: &str) -> keyring::Result<keyring::Entry> {
        keyring::Entry::new(&format!("{SERVICE}.{kind}"), &account.to_lowercase())
    }

    fn get(kind: &str, account: &str) -> keyring::Result<Option<String>> {
        match entry(kind, account)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn set(kind: &str, account: &str, value: Option<&str>) -> keyring::Result<()> {
        let e = entry(kind, account)?;
        match value {
            Some(v) if !v.is_empty() => e.set_password(v),
            _ => match e.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(err) => Err(err),
            },
        }
    }

    /// (refresh token, Steam Guard data).
    pub fn load(account: &str) -> keyring::Result<(Option<String>, Option<String>)> {
        Ok((get("refresh-token", account)?, get("guard-data", account)?))
    }

    pub fn save(acc: &SavedAccount) -> keyring::Result<()> {
        set("refresh-token", &acc.account_name, Some(&acc.refresh_token))?;
        set("guard-data", &acc.account_name, acc.guard_data.as_deref())
    }

    pub fn delete(account: &str) -> keyring::Result<()> {
        set("refresh-token", account, None)?;
        set("guard-data", account, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_impls_redact_passwords_codes_keys_and_tokens() {
        let login = crate::Command::LoginPassword { account: "alice".into(), password: "super_secret_pw".into() };
        let login_dbg = format!("{login:?}");
        assert!(login_dbg.contains("alice"));
        assert!(!login_dbg.contains("super_secret_pw"), "password leaked in Command Debug: {login_dbg}");

        let code = crate::Command::SubmitCode("987654".into());
        assert!(!format!("{code:?}").contains("987654"), "Steam Guard code leaked in Command Debug");

        let key = crate::Command::ActivateKey { key: "AAAAA-BBBBB-CCCCC".into() };
        assert!(!format!("{key:?}").contains("AAAAA-BBBBB-CCCCC"), "CD key leaked in Command Debug");

        let saved = SavedAccount {
            account_name: "alice".into(),
            steam_id: 76561198000000001,
            persona_name: "Alice".into(),
            avatar_hash: None,
            refresh_token: "eyJhbGciOi.secret_refresh".into(),
            guard_data: Some("guard_machine_secret".into()),
        };
        let saved_dbg = format!("{saved:?}");
        assert!(saved_dbg.contains("alice"));
        assert!(!saved_dbg.contains("secret_refresh"), "refresh_token leaked in SavedAccount Debug: {saved_dbg}");
        assert!(!saved_dbg.contains("guard_machine_secret"), "guard_data leaked in SavedAccount Debug: {saved_dbg}");
    }

    #[cfg(unix)]
    #[test]
    fn write_private_uses_excl_temp_and_0700_0600_perms() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("sir-store-sec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sub = dir.join("cfg").join("offline").join("123");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        let target = sub.join("account.json");
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, "untouched").unwrap();
        std::os::unix::fs::symlink(&victim, target.with_extension("tmp")).unwrap();

        write_private(&target, b"{\"ok\":true}").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "untouched", "predictable .tmp symlink was followed!");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"ok\":true}");
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&sub).unwrap().permissions().mode() & 0o777, 0o700);

        let src_save = dir.join("save.dat");
        std::fs::write(&src_save, b"save-bytes").unwrap();
        std::fs::set_permissions(&src_save, std::fs::Permissions::from_mode(0o644)).unwrap();
        let bkp = dir.join("cfg").join("cloud-backups").join("570").join("99").join("save.dat");
        copy_private(&src_save, &bkp).unwrap();
        assert_eq!(std::fs::metadata(&bkp).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(bkp.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);

        let off = dir.join("cfg").join("offline").join("123").join("playtime.json");
        crate::offline::write(&off, &vec![1u32, 2, 3]);
        assert_eq!(std::fs::metadata(&off).unwrap().permissions().mode() & 0o777, 0o600);

        let fb = resolve_config_dir(None);
        assert_eq!(fb, PathBuf::from(format!("/tmp/steaminrust-{}", current_uid())));
        assert_eq!(std::fs::metadata(&fb).unwrap().permissions().mode() & 0o777, 0o700);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
