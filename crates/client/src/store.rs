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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "SteamInRust", "SteamInRust")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
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
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(list) {
        Ok(json) => {
            if let Err(e) = write_private(&p, json.as_bytes()) {
                tracing::error!("could not save accounts: {e}");
            }
        }
        Err(e) => tracing::error!("could not encode accounts: {e}"),
    }
}

/// Replace `path` with `bytes`, mode 0600 on Unix (also fixing an older,
/// world-readable file).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    {
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
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
