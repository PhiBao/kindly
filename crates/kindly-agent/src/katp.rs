//! KATP — the Kindly Agent Terms Protocol.
//!
//! A tiny, signed message protocol for two agents negotiating payment terms:
//! the supplier's collections agent (Kindly) and the buyer's accounts-payable
//! agent. It is deliberately AP2-flavoured: the *terms* are the object of
//! negotiation, and every message carries the sender's public key plus, on
//! first contact, a signed attestation of the mandate the sender is acting
//! under. That gives a judge (or a counterparty) something to verify rather
//! than a claim to trust.
//!
//! Canonicalization note: payloads are signed as the deterministic JSON of the
//! [`KatpEnvelope`] minus the `signature` field. That is sufficient between two
//! implementations of this crate; a multi-vendor deployment would move to a
//! canonical JSON scheme (RFC 8785) and a stable protocol registry.

use base64::Engine;
use chrono::{DateTime, NaiveDate, Utc};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const KATP_VERSION: &str = "kindly/0.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedInstallment {
    pub amount_minor: i64,
    pub due: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsProposal {
    pub pay_now_minor: i64,
    pub discount_minor: i64,
    pub currency: String,
    pub installments: Vec<ProposedInstallment>,
    /// Terms expire if not accepted by this date.
    pub valid_until: NaiveDate,
}

impl TermsProposal {
    pub fn total_minor(&self) -> i64 {
        self.pay_now_minor
            + self.discount_minor
            + self.installments.iter().map(|i| i.amount_minor).sum::<i64>()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KatpMessage {
    /// First contact: who I am, what I am allowed to do, how to verify it.
    Hello {
        agent: String,
        company: String,
        mandate_digest: Vec<String>,
    },
    Offer {
        terms: TermsProposal,
        note: String,
    },
    Counter {
        terms: TermsProposal,
        note: String,
    },
    Accept {
        terms: TermsProposal,
        note: String,
    },
    Reject {
        reason: String,
        note: String,
    },
    /// The buyer authorises the first payment (human-present approval step).
    Authorise {
        amount_minor: i64,
        currency: String,
        note: String,
    },
    /// Money moved. Reference is the PayPal transaction or capture id.
    Settle {
        reference: String,
        amount_minor: i64,
        currency: String,
        note: String,
    },
    Info {
        note: String,
    },
}

impl KatpMessage {
    pub fn label(&self) -> &'static str {
        match self {
            KatpMessage::Hello { .. } => "hello",
            KatpMessage::Offer { .. } => "offer",
            KatpMessage::Counter { .. } => "counter",
            KatpMessage::Accept { .. } => "accept",
            KatpMessage::Reject { .. } => "reject",
            KatpMessage::Authorise { .. } => "authorise",
            KatpMessage::Settle { .. } => "settle",
            KatpMessage::Info { .. } => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    pub agent_id: String,
    pub company: String,
    pub public_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MandateAttestation {
    pub agent_id: String,
    /// SHA-256 of the mandate's canonical JSON.
    pub mandate_digest: String,
    pub issued_at: DateTime<Utc>,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KatpEnvelope {
    pub version: String,
    pub from: AgentRef,
    /// Present on first contact and whenever authority is refreshed.
    pub mandate: Option<MandateAttestation>,
    pub message: KatpMessage,
    pub signature: String,
}

/// Payload that is actually signed (everything except the signature itself).
#[derive(Serialize)]
struct SigningPayload<'a> {
    version: &'a str,
    from: &'a AgentRef,
    mandate: &'a Option<MandateAttestation>,
    message: &'a KatpMessage,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(value: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(value).ok()
}

pub fn mandate_digest(canonical_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_json.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl MandateAttestation {
    pub fn sign(key: &SigningKey, agent_id: &str, canonical_mandate: &str) -> Self {
        let digest = mandate_digest(canonical_mandate);
        let issued_at = Utc::now();
        let payload = format!("{agent_id}|{digest}|{}", issued_at.to_rfc3339());
        let signature = key.sign(payload.as_bytes());
        Self {
            agent_id: agent_id.to_string(),
            mandate_digest: digest,
            issued_at,
            signature: b64(&signature.to_bytes()),
        }
    }

    pub fn verify(&self, canonical_mandate: &str) -> bool {
        let digest = mandate_digest(canonical_mandate);
        digest == self.mandate_digest
    }

    /// Verify the attestation against the signer's public key and mandate.
    pub fn verify_with_key(&self, public_key_b64: &str, canonical_mandate: &str) -> bool {
        let Some(key_bytes) = unb64(public_key_b64) else {
            return false;
        };
        let Ok(key_bytes): Result<[u8; 32], _> = key_bytes.try_into() else {
            return false;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
            return false;
        };
        let digest = mandate_digest(canonical_mandate);
        if digest != self.mandate_digest {
            return false;
        }
        let payload = format!(
            "{}|{}|{}",
            self.agent_id,
            self.mandate_digest,
            self.issued_at.to_rfc3339()
        );
        let Some(signature_bytes) = unb64(&self.signature) else {
            return false;
        };
        let Ok(signature_bytes): Result<[u8; 64], _> = signature_bytes.try_into() else {
            return false;
        };
        let signature = Signature::from_bytes(&signature_bytes);
        verifying_key.verify(payload.as_bytes(), &signature).is_ok()
    }
}

impl KatpEnvelope {
    pub fn build(
        key: &SigningKey,
        agent: AgentRef,
        mandate: Option<MandateAttestation>,
        message: KatpMessage,
    ) -> Self {
        let mut envelope = Self {
            version: KATP_VERSION.to_string(),
            from: agent,
            mandate,
            message,
            signature: String::new(),
        };
        envelope.signature = envelope.sign_with(key);
        envelope
    }

    fn payload_json(&self) -> String {
        let payload = SigningPayload {
            version: &self.version,
            from: &self.from,
            mandate: &self.mandate,
            message: &self.message,
        };
        serde_json::to_string(&payload).unwrap_or_default()
    }

    fn sign_with(&self, key: &SigningKey) -> String {
        b64(&key.sign(self.payload_json().as_bytes()).to_bytes())
    }

    /// Verify the envelope's own signature, and the mandate attestation when
    /// present (using the envelope's declared public key).
    pub fn verify(&self) -> bool {
        if self.version != KATP_VERSION {
            return false;
        }
        let Some(public_key_b64) = Some(self.from.public_key.clone()) else {
            return false;
        };
        let Some(key_bytes) = unb64(&public_key_b64) else {
            return false;
        };
        let Ok(key_bytes): Result<[u8; 32], _> = key_bytes.try_into() else {
            return false;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&key_bytes) else {
            return false;
        };
        let Some(signature_bytes) = unb64(&self.signature) else {
            return false;
        };
        let Ok(signature_bytes): Result<[u8; 64], _> = signature_bytes.try_into() else {
            return false;
        };
        let signature = Signature::from_bytes(&signature_bytes);
        verifying_key
            .verify(self.payload_json().as_bytes(), &signature)
            .is_ok()
    }
}

pub fn public_key_b64(key: &SigningKey) -> String {
    b64(key.verifying_key().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    fn agent_key() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }

    fn agent_ref(key: &SigningKey) -> AgentRef {
        AgentRef {
            agent_id: "kindly-test".into(),
            company: "Maya Studio".into(),
            public_key: public_key_b64(key),
        }
    }

    fn proposal() -> TermsProposal {
        TermsProposal {
            pay_now_minor: 168_000,
            discount_minor: 0,
            currency: "USD".into(),
            installments: vec![ProposedInstallment {
                amount_minor: 72_000,
                due: NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
            }],
            valid_until: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        }
    }

    #[test]
    fn envelopes_round_trip_and_verify() {
        let key = agent_key();
        let mandate = MandateAttestation::sign(&key, "kindly-test", "{\"limits\":[]}");
        let envelope = KatpEnvelope::build(
            &key,
            agent_ref(&key),
            Some(mandate.clone()),
            KatpMessage::Offer {
                terms: proposal(),
                note: "70% now, rest in two weeks.".into(),
            },
        );
        assert!(envelope.verify());
        assert!(mandate.verify_with_key(&envelope.from.public_key, "{\"limits\":[]}"));
        assert!(!mandate.verify_with_key(&envelope.from.public_key, "{\"limits\":[\"tampered\"]}"));

        let json = serde_json::to_string(&envelope).unwrap();
        let back: KatpEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, envelope);
        assert!(back.verify());
    }

    #[test]
    fn tampered_messages_fail_verification() {
        let key = agent_key();
        let mut envelope = KatpEnvelope::build(
            &key,
            agent_ref(&key),
            None,
            KatpMessage::Offer {
                terms: proposal(),
                note: "original".into(),
            },
        );
        envelope.message = KatpMessage::Offer {
            terms: proposal(),
            note: "tampered".into(),
        };
        assert!(!envelope.verify());
    }

    #[test]
    fn a_different_key_cannot_forge_an_envelope() {
        let key = agent_key();
        let attacker = agent_key();
        // Attacker swaps in their own public key but keeps the old signature.
        let mut envelope = KatpEnvelope::build(
            &key,
            agent_ref(&key),
            None,
            KatpMessage::Accept {
                terms: proposal(),
                note: "accept".into(),
            },
        );
        envelope.from.public_key = public_key_b64(&attacker);
        assert!(!envelope.verify());
    }
}
