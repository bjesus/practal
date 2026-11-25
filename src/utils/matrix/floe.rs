//! Support for downloading and decrypting Matrix media, including MSC4571 /
//! FLOE (`com.beeper.floe.v1` / `v3`) encrypted attachments.

use std::io;

use matrix_sdk::{
    Client,
    media::{MediaFormat, MediaRequestParameters},
};
use ruma::{
    events::room::{EncryptedFile, MediaSource},
    serde::Base64,
};
use tracing::warn;

/// Fetch media content, transparently decrypting MSC4571 / FLOE
/// (`com.beeper.floe.v1` / `v3`) encrypted attachments that `matrix-sdk`
/// does not yet support natively.
pub(crate) async fn get_media_content(
    client: &Client,
    settings: &MediaRequestParameters,
    use_cache: bool,
) -> Result<Vec<u8>, matrix_sdk::Error> {
    let media = client.media();

    if let MediaSource::Encrypted(file) = &settings.source
        && is_floe_version(file.info.version())
    {
        let raw_settings = MediaRequestParameters {
            source: MediaSource::Plain(file.url.clone()),
            format: MediaFormat::File,
        };
        let raw_body = media.get_media_content(&raw_settings, use_cache).await?;

        return decrypt_floe_file(file, &raw_body)
            .map_err(|err| matrix_sdk::Error::UnknownError(Box::new(err)));
    }

    match media.get_media_content(settings, use_cache).await {
        Ok(data) => Ok(data),
        Err(err) => {
            if let MediaSource::Encrypted(file) = &settings.source {
                let data = file.info.data();
                let keys: Vec<&str> = data.keys().map(String::as_str).collect();
                warn!(
                    version = file.info.version(),
                    ?keys,
                    has_hashes = !file.hashes.is_empty(),
                    "Failed to fetch/decrypt encrypted media: {err}"
                );
            }
            Err(err)
        }
    }
}

/// Whether the given `EncryptedFile` version string is a FLOE version.
fn is_floe_version(version: &str) -> bool {
    matches!(version, "com.beeper.floe.v1" | "v3")
}

/// Decode an unpadded or padded Base64 string (standard or URL-safe alphabet).
fn decode_base64_lenient(encoded: &str) -> Result<Vec<u8>, io::Error> {
    let trimmed = encoded.trim_end_matches('=');
    if let Ok(decoded) = Base64::<ruma::serde::base64::Standard>::parse(trimmed) {
        return Ok(decoded.into_inner());
    }
    if let Ok(decoded) = Base64::<ruma::serde::base64::UrlSafe>::parse(trimmed) {
        return Ok(decoded.into_inner());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid Base64 in FLOE attachment metadata",
    ))
}

/// Decrypt a FLOE-encrypted attachment (`com.beeper.floe.v1` / `v3`).
fn decrypt_floe_file(file: &EncryptedFile, body: &[u8]) -> Result<Vec<u8>, io::Error> {
    let data = file.info.data();

    let key_b64 = data
        .get("floe_key")
        .or_else(|| data.get("key"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            let keys: Vec<&str> = data.keys().map(String::as_str).collect();
            warn!(
                version = file.info.version(),
                ?keys,
                "FLOE encrypted file is missing `floe_key`"
            );
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing `floe_key` in FLOE attachment",
            )
        })?;

    let header_b64 = data
        .get("floe_header")
        .or_else(|| data.get("header"))
        .and_then(|v| v.as_str());

    let key_bytes = decode_base64_lenient(key_b64)?;
    let key_array: [u8; fast_floe::Key::LEN] = key_bytes.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid FLOE key length (expected 32 bytes)",
        )
    })?;
    let key = fast_floe::Key::from_bytes(key_array);

    let header_bytes = match header_b64 {
        Some(h) => Some(decode_base64_lenient(h)?),
        None => None,
    };

    decrypt_floe_bytes(&key, header_bytes.as_deref(), body)
}

/// Decrypt FLOE ciphertext given the key, optional detached header, and body.
fn decrypt_floe_bytes(
    key: &fast_floe::Key,
    header: Option<&[u8]>,
    body: &[u8],
) -> Result<Vec<u8>, io::Error> {
    let plaintext = match header {
        Some(hdr) if !body.starts_with(hdr) => {
            let mut combined = Vec::with_capacity(hdr.len() + body.len());
            combined.extend_from_slice(hdr);
            combined.extend_from_slice(body);
            fast_floe::decrypt(key, b"", &combined)
        }
        _ => fast_floe::decrypt(key, b"", body),
    };

    plaintext.map_err(|err| io::Error::other(format!("FLOE decryption failed: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_floe_detached_header() {
        let key = fast_floe::Key::generate().unwrap();
        let params = fast_floe::Parameters::with_segment_length(4096).unwrap();
        let original = b"hello beeper floe image bytes";

        let full_ciphertext = fast_floe::encrypt(&key, b"", params, original).unwrap();
        // Verify both combined and detached header forms work.
        let decrypted_combined = decrypt_floe_bytes(&key, None, &full_ciphertext).unwrap();
        assert_eq!(decrypted_combined, original);

        // Test splitting at fast_floe::Header::LEN.
        let header_len = fast_floe::Header::LEN;
        let (hdr, body) = full_ciphertext.split_at(header_len);
        let decrypted_detached = decrypt_floe_bytes(&key, Some(hdr), body).unwrap();
        assert_eq!(decrypted_detached, original);
    }

    #[test]
    fn deserialize_floe_image_without_hashes_and_decrypt() {
        use ruma::events::room::message::{MessageType, RoomMessageEventContent};

        let key_bytes = [7u8; fast_floe::Key::LEN];
        let key = fast_floe::Key::from_bytes(key_bytes);
        let params = fast_floe::Parameters::with_segment_length(4096).unwrap();
        let original = b"fake-jpeg-bytes-from-beeper";

        let full_ciphertext = fast_floe::encrypt(&key, b"", params, original).unwrap();
        let (hdr, body) = full_ciphertext.split_at(fast_floe::Header::LEN);

        let key_b64 = Base64::<ruma::serde::base64::Standard>::new(key_bytes.to_vec()).encode();
        let hdr_b64 = Base64::<ruma::serde::base64::Standard>::new(hdr.to_vec()).encode();

        let event_json = serde_json::json!({
            "msgtype": "m.image",
            "body": "photo.jpg",
            "file": {
                "url": "mxc://beeper.local/abc123",
                "v": "com.beeper.floe.v1",
                "floe_key": key_b64,
                "floe_header": hdr_b64
            },
            "info": {
                "mimetype": "image/jpeg",
                "w": 800,
                "h": 600
            }
        });

        let content: RoomMessageEventContent = serde_json::from_value(event_json)
            .expect("FLOE image without hashes should deserialize");

        let MessageType::Image(image) = content.msgtype else {
            panic!("expected m.image");
        };
        let MediaSource::Encrypted(file) = image.source else {
            panic!("expected encrypted media source");
        };

        assert_eq!(file.info.version(), "com.beeper.floe.v1");
        assert!(file.hashes.is_empty());

        let decrypted = decrypt_floe_file(&file, body).expect("FLOE decryption should succeed");
        assert_eq!(decrypted, original);
    }
}
