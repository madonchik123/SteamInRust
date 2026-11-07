//! Saved logins ("remember me"), stored in the user's config folder.
//!
//! The refresh token is a credential. It is kept in a per-user file for now;
//! moving it to the OS keychain is on the roadmap.

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
    pub refresh_token: String,
    #[serde(default)]
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

pub fn load() -> Vec<SavedAccount> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(list: &[SavedAccount]) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(list) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&p, json) {
                tracing::error!("could not save accounts: {e}");
            }
        }
        Err(e) => tracing::error!("could not encode accounts: {e}"),
    }
}

pub fn find(account: &str) -> Option<SavedAccount> {
    load().into_iter().find(|a| a.account_name.eq_ignore_ascii_case(account))
}

/// Insert or update by account name, keeping fields the new value leaves empty.
pub fn upsert(mut acc: SavedAccount) {
    let mut list = load();
    if let Some(old) = list.iter().find(|a| a.account_name.eq_ignore_ascii_case(&acc.account_name)) {
        if acc.persona_name.is_empty() {
            acc.persona_name = old.persona_name.clone();
        }
        if acc.guard_data.is_none() {
            acc.guard_data = old.guard_data.clone();
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

pub fn remove(account: &str) {
    let mut list = load();
    list.retain(|a| !a.account_name.eq_ignore_ascii_case(account));
    write(&list);
}
