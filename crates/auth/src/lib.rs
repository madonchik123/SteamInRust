//! Steam's modern login flow (the `Authentication` service).
//!
//! 1. [`begin`]: fetch the account's RSA key, encrypt the password, start a session.
//! 2. If [`PendingAuth::guard`] asks for a code, call [`PendingAuth::submit_code`].
//!    If it allows phone approval, just keep polling.
//! 3. [`PendingAuth::poll`] until it returns [`Tokens`].
//!
//! The refresh token can be saved and reused with `ClientLogon` to skip the
//! password next time (that's how "remember me" works).

use base64::Engine;
use net::Connection;
use proto::*;
use rsa::{BigUint, Pkcs1v15Encrypt, RsaPublicKey};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Net(#[from] net::Error),
    #[error("could not encrypt password: {0}")]
    Crypto(String),
    #[error("Steam sent an invalid public key")]
    BadKey,
}

pub type Result<T> = std::result::Result<T, Error>;

/// What Steam Guard wants from the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guard {
    /// No extra step; poll right away.
    None,
    /// A code from the Steam Mobile app.
    DeviceCode,
    /// A code sent to the account's email, with the email domain as a hint.
    EmailCode { hint: String },
    /// Tap "approve" in the mobile app.
    DeviceConfirmation,
    /// Click the link in an email.
    EmailConfirmation,
}

impl Guard {
    fn from_proto(c: &CAuthenticationAllowedConfirmation) -> Option<Self> {
        Some(match c.confirmation_type() {
            EAuthSessionGuardType::KEAuthSessionGuardTypeNone => Guard::None,
            EAuthSessionGuardType::KEAuthSessionGuardTypeDeviceCode => Guard::DeviceCode,
            EAuthSessionGuardType::KEAuthSessionGuardTypeEmailCode => Guard::EmailCode {
                hint: c.associated_message().to_string(),
            },
            EAuthSessionGuardType::KEAuthSessionGuardTypeDeviceConfirmation => {
                Guard::DeviceConfirmation
            }
            EAuthSessionGuardType::KEAuthSessionGuardTypeEmailConfirmation => {
                Guard::EmailConfirmation
            }
            _ => return None,
        })
    }

    pub fn needs_code(&self) -> bool {
        matches!(self, Guard::DeviceCode | Guard::EmailCode { .. })
    }
}

#[derive(Debug, Clone)]
pub struct Tokens {
    pub account_name: String,
    pub steam_id: u64,
    pub refresh_token: String,
    pub access_token: String,
    /// Lets Steam skip email codes on this machine next time.
    pub guard_data: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PendingAuth {
    client_id: u64,
    request_id: Vec<u8>,
    pub steam_id: u64,
    pub interval: Duration,
    /// Options Steam allows, in its preferred order.
    pub guards: Vec<Guard>,
    /// For QR logins: the URL to encode. It can change while polling.
    pub challenge_url: Option<String>,
}

/// Start a QR-code login. Show `challenge_url` as a QR code, scan it with the
/// Steam Mobile app, then [`PendingAuth::poll`] as usual.
pub async fn begin_qr(conn: &Connection, device: &Device) -> Result<PendingAuth> {
    let platform = EAuthTokenPlatformType::KEAuthTokenPlatformTypeSteamClient as i32;
    let resp: CAuthenticationBeginAuthSessionViaQrResponse = conn
        .call(
            "Authentication.BeginAuthSessionViaQR#1",
            &CAuthenticationBeginAuthSessionViaQrRequest {
                device_friendly_name: Some(device.name.clone()),
                platform_type: Some(platform),
                website_id: Some("Client".into()),
                device_details: Some(CAuthenticationDeviceDetails {
                    device_friendly_name: Some(device.name.clone()),
                    platform_type: Some(platform),
                    os_type: Some(OS_TYPE_WIN11),
                    ..Default::default()
                }),
            },
        )
        .await?;
    Ok(PendingAuth {
        client_id: resp.client_id(),
        request_id: resp.request_id().to_vec(),
        steam_id: 0,
        interval: Duration::from_secs_f32(resp.interval().clamp(1.0, 10.0)),
        guards: resp.allowed_confirmations.iter().filter_map(Guard::from_proto).collect(),
        challenge_url: Some(resp.challenge_url().to_string()),
    })
}

/// Info about this device shown in the user's "authorized devices" list.
#[derive(Debug, Clone)]
pub struct Device {
    pub name: String,
}

impl Default for Device {
    fn default() -> Self {
        let name = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "SteamInRust".into());
        Self { name }
    }
}

const OS_TYPE_WIN11: i32 = 20;

pub async fn begin(
    conn: &Connection,
    account_name: &str,
    password: &str,
    guard_data: Option<String>,
    device: &Device,
) -> Result<PendingAuth> {
    let key: CAuthenticationGetPasswordRsaPublicKeyResponse = conn
        .call(
            "Authentication.GetPasswordRSAPublicKey#1",
            &CAuthenticationGetPasswordRsaPublicKeyRequest {
                account_name: Some(account_name.into()),
            },
        )
        .await?;
    let encrypted = encrypt_password(key.publickey_mod(), key.publickey_exp(), password)?;

    let platform = EAuthTokenPlatformType::KEAuthTokenPlatformTypeSteamClient as i32;
    let resp: CAuthenticationBeginAuthSessionViaCredentialsResponse = conn
        .call(
            "Authentication.BeginAuthSessionViaCredentials#1",
            &CAuthenticationBeginAuthSessionViaCredentialsRequest {
                device_friendly_name: Some(device.name.clone()),
                account_name: Some(account_name.into()),
                encrypted_password: Some(encrypted),
                encryption_timestamp: key.timestamp,
                remember_login: Some(true),
                platform_type: Some(platform),
                persistence: Some(ESessionPersistence::KESessionPersistencePersistent as i32),
                website_id: Some("Client".into()),
                device_details: Some(CAuthenticationDeviceDetails {
                    device_friendly_name: Some(device.name.clone()),
                    platform_type: Some(platform),
                    os_type: Some(OS_TYPE_WIN11),
                    ..Default::default()
                }),
                guard_data,
                ..Default::default()
            },
        )
        .await?;

    Ok(PendingAuth {
        client_id: resp.client_id(),
        request_id: resp.request_id().to_vec(),
        steam_id: resp.steamid(),
        interval: Duration::from_secs_f32(resp.interval().clamp(1.0, 10.0)),
        guards: resp.allowed_confirmations.iter().filter_map(Guard::from_proto).collect(),
        challenge_url: None,
    })
}

fn encrypt_password(modulus_hex: &str, exponent_hex: &str, password: &str) -> Result<String> {
    let n = BigUint::from_bytes_be(&hex(modulus_hex).ok_or(Error::BadKey)?);
    let e = BigUint::from_bytes_be(&hex(exponent_hex).ok_or(Error::BadKey)?);
    let key = RsaPublicKey::new(n, e).map_err(|e| Error::Crypto(e.to_string()))?;
    let bytes = key
        .encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, password.as_bytes())
        .map_err(|e| Error::Crypto(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn hex(s: &str) -> Option<Vec<u8>> {
    let s = if s.len() % 2 == 1 { format!("0{s}") } else { s.to_string() };
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

impl PendingAuth {
    /// The first option Steam suggests.
    pub fn preferred(&self) -> Guard {
        self.guards.first().cloned().unwrap_or(Guard::None)
    }

    /// The code-based option, if any (the user may type a code even when phone
    /// approval is preferred).
    pub fn code_guard(&self) -> Option<&Guard> {
        self.guards.iter().find(|g| g.needs_code())
    }

    pub async fn submit_code(&self, conn: &Connection, code: &str) -> Result<()> {
        let kind = match self.code_guard() {
            Some(Guard::EmailCode { .. }) => EAuthSessionGuardType::KEAuthSessionGuardTypeEmailCode,
            _ => EAuthSessionGuardType::KEAuthSessionGuardTypeDeviceCode,
        };
        let _: CAuthenticationUpdateAuthSessionWithSteamGuardCodeResponse = conn
            .call(
                "Authentication.UpdateAuthSessionWithSteamGuardCode#1",
                &CAuthenticationUpdateAuthSessionWithSteamGuardCodeRequest {
                    client_id: Some(self.client_id),
                    steamid: Some(self.steam_id),
                    code: Some(code.trim().to_uppercase()),
                    code_type: Some(kind as i32),
                },
            )
            .await?;
        Ok(())
    }

    /// `Ok(None)` means "not approved yet, poll again after `interval`".
    pub async fn poll(&mut self, conn: &Connection) -> Result<Option<Tokens>> {
        let r: CAuthenticationPollAuthSessionStatusResponse = conn
            .call(
                "Authentication.PollAuthSessionStatus#1",
                &CAuthenticationPollAuthSessionStatusRequest {
                    client_id: Some(self.client_id),
                    request_id: Some(self.request_id.clone()),
                    token_to_revoke: None,
                },
            )
            .await?;
        if let Some(id) = r.new_client_id {
            self.client_id = id;
        }
        if let Some(url) = r.new_challenge_url.clone().filter(|u| !u.is_empty()) {
            self.challenge_url = Some(url);
        }
        if r.refresh_token().is_empty() {
            return Ok(None);
        }
        // QR logins learn the SteamID from the token itself.
        let steam_id = if self.steam_id != 0 { self.steam_id } else { steam_id_from_jwt(r.refresh_token()).unwrap_or(0) };
        Ok(Some(Tokens {
            account_name: r.account_name().to_string(),
            steam_id,
            refresh_token: r.refresh_token().to_string(),
            access_token: r.access_token().to_string(),
            guard_data: r.new_guard_data.filter(|g| !g.is_empty()),
        }))
    }
}

/// Read the `sub` claim (the SteamID) from a JWT without verifying it.
/// Only used to learn our own id; Steam validates the token itself.
fn steam_id_from_jwt(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let start = text.find("\"sub\"")? + 5;
    let rest = &text[start..];
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_sub_from_jwt() {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"iss":"steam","sub":"76561197960287930","aud":["client"]}"#);
        let token = format!("eyJhbGciOiJFZERTQSJ9.{payload}.sig");
        assert_eq!(super::steam_id_from_jwt(&token), Some(76561197960287930));
    }

    #[test]
    fn hex_decodes_odd_length() {
        assert_eq!(super::hex("10001"), Some(vec![0x01, 0x00, 0x01]));
        assert_eq!(super::hex("zz"), None);
    }
}
