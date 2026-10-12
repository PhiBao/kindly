//! Client-portal capability tokens.
//!
//! The portal is public (the client has no account), so access is granted by a
//! signed, expiring token in the URL — the same pattern as PayPal's own payer
//! view links. Tokens are HMAC-SHA256 over `invoice_id|expiry`, so a portal
//! link cannot be forged, cannot be replayed after expiry, and cannot be used
//! to reach another invoice.

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use kindly_core::ids::InvoiceId;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub const PORTAL_TOKEN_TTL_DAYS: i64 = 45;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(value: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(value).ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalTokenError {
    Malformed,
    BadSignature,
    Expired,
}

pub fn sign_portal_token(secret: &str, invoice_id: InvoiceId, ttl_days: i64) -> String {
    let expires = Utc::now() + Duration::days(ttl_days);
    let expiry = expires.timestamp().to_string();
    let id = invoice_id.to_string();
    let payload = format!("{id}.{expiry}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(payload.as_bytes());
    let signature = b64(&mac.finalize().into_bytes());
    format!("{payload}.{signature}")
}

/// Verify a token and return the invoice id it grants access to.
pub fn verify_portal_token(
    secret: &str,
    token: &str,
    now: DateTime<Utc>,
) -> Result<InvoiceId, PortalTokenError> {
    let mut parts = token.split('.');
    let id = parts.next().ok_or(PortalTokenError::Malformed)?;
    let expiry = parts.next().ok_or(PortalTokenError::Malformed)?;
    let signature = parts.next().ok_or(PortalTokenError::Malformed)?;
    if parts.next().is_some() {
        return Err(PortalTokenError::Malformed);
    }

    let payload = format!("{id}.{expiry}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(payload.as_bytes());
    let expected = mac.finalize().into_bytes();
    let provided = unb64(signature).ok_or(PortalTokenError::Malformed)?;
    if !constant_time_eq(&expected, &provided) {
        return Err(PortalTokenError::BadSignature);
    }

    let expiry_ts: i64 = expiry.parse().map_err(|_| PortalTokenError::Malformed)?;
    let expires_at = DateTime::from_timestamp(expiry_ts, 0).ok_or(PortalTokenError::Malformed)?;
    if now > expires_at {
        return Err(PortalTokenError::Expired);
    }

    let id = InvoiceId::from_uuid(id.parse().map_err(|_| PortalTokenError::Malformed)?);
    Ok(id)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip() {
        let secret = "test-secret";
        let invoice = InvoiceId::new();
        let token = sign_portal_token(secret, invoice, 7);
        let parsed = verify_portal_token(secret, &token, Utc::now()).unwrap();
        assert_eq!(parsed, invoice);
    }

    #[test]
    fn forged_and_expired_tokens_are_rejected() {
        let secret = "test-secret";
        let invoice = InvoiceId::new();
        let token = sign_portal_token(secret, invoice, 7);

        // Wrong secret
        assert_eq!(
            verify_portal_token("other-secret", &token, Utc::now()),
            Err(PortalTokenError::BadSignature)
        );

        // Tampered id
        let mut parts: Vec<&str> = token.split('.').collect();
        parts[0] = "00000000-0000-0000-0000-000000000000";
        let tampered = parts.join(".");
        assert!(verify_portal_token(secret, &tampered, Utc::now()).is_err());

        // Expired
        let expired = sign_portal_token(secret, invoice, -1);
        assert_eq!(
            verify_portal_token(secret, &expired, Utc::now()),
            Err(PortalTokenError::Expired)
        );

        assert_eq!(
            verify_portal_token(secret, "not-a-token", Utc::now()),
            Err(PortalTokenError::Malformed)
        );
    }
}
