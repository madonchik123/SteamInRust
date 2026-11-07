//! Steam's "symmetric" encryption for chunks and manifest filenames:
//! the first 16 bytes are the IV, encrypted with AES-256-ECB; the rest is
//! AES-256-CBC with PKCS#7 padding.

use crate::{Error, Result};
use aes::Aes256;
use aes::cipher::{BlockDecryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7, generic_array::GenericArray};
use base64::Engine;

type CbcDec = cbc::Decryptor<Aes256>;
type EcbDec = ecb::Decryptor<Aes256>;

pub fn symmetric_decrypt(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    if key.len() != 32 {
        return Err(Error::Crypto("depot key must be 32 bytes".into()));
    }
    if data.len() < 32 || data.len() % 16 != 0 {
        return Err(Error::Crypto("ciphertext has a bad length".into()));
    }
    let mut iv = GenericArray::clone_from_slice(&data[..16]);
    EcbDec::new_from_slice(key)
        .map_err(|e| Error::Crypto(e.to_string()))?
        .decrypt_block_mut(&mut iv);
    let mut body = data[16..].to_vec();
    let plain = CbcDec::new_from_slices(key, &iv)
        .map_err(|e| Error::Crypto(e.to_string()))?
        .decrypt_padded_mut::<Pkcs7>(&mut body)
        .map_err(|e| Error::Crypto(e.to_string()))?
        .to_vec();
    Ok(plain)
}

/// Filenames in CDN manifests are base64 of encrypted, NUL-terminated text.
pub fn decrypt_filename(b64: &str, key: &[u8]) -> Result<String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim().replace('\n', ""))
        .map_err(|e| Error::Crypto(e.to_string()))?;
    let plain = symmetric_decrypt(&raw, key)?;
    let end = plain.iter().position(|b| *b == 0).unwrap_or(plain.len());
    Ok(String::from_utf8_lossy(&plain[..end]).into_owned())
}

#[cfg(test)]
pub(crate) fn symmetric_encrypt_for_tests(plain: &[u8], key: &[u8], iv: [u8; 16]) -> Vec<u8> {
    use aes::cipher::{BlockEncryptMut, block_padding::Pkcs7};
    type CbcEnc = cbc::Encryptor<Aes256>;
    type EcbEnc = ecb::Encryptor<Aes256>;
    let mut enc_iv = GenericArray::clone_from_slice(&iv);
    EcbEnc::new_from_slice(key).unwrap().encrypt_block_mut(&mut enc_iv);
    let mut buf = vec![0u8; plain.len() + 16];
    buf[..plain.len()].copy_from_slice(plain);
    let body = CbcEnc::new_from_slices(key, &iv).unwrap().encrypt_padded_mut::<Pkcs7>(&mut buf, plain.len()).unwrap().to_vec();
    let mut out = enc_iv.to_vec();
    out.extend(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let data = symmetric_encrypt_for_tests(b"hello depot", &key, [3u8; 16]);
        assert_eq!(symmetric_decrypt(&data, &key).unwrap(), b"hello depot");
    }
}
