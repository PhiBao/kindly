//! The buyer's side: an accounts-payable agent that negotiates with Kindly.
//!
//! For the agent-to-agent story to be real, *both* sides must be real code.
//! This agent represents a client company's finance team: it holds a cash
//! policy, signs its messages, and negotiates within limits — it is not a
//! scripted responder. In production this role would be played by the client's
//! own AP system or a third-party agent; in the demo it is run by Kindly's
//! runtime as a separate, independently-keyed agent with its own mandate.
//!
//! When a plan exceeds what the agent may authorise alone, it stops and asks
//! its human for approval — mirroring AP2's "human present" flow, and giving
//! the demo its most important trust moment.

use crate::katp::{
    public_key_b64, AgentRef, KatpEnvelope, KatpMessage, MandateAttestation, ProposedInstallment,
    TermsProposal,
};
use crate::llm::{complete_json, LlmProvider, LlmRequest};
use crate::prompts;
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use serde::Deserialize;

/// The policy the buyer's agent operates under.
#[derive(Debug, Clone)]
pub struct BuyerMandate {
    pub company: String,
    pub contact_name: String,
    pub agent_id: String,
    /// Human-readable policy lines, shown in the transcript and the UI.
    pub policy: Vec<String>,
    /// Most cash the agent can release without waking a human.
    pub max_unapproved_cash_minor: i64,
    /// Most cash it would ever propose, even with approval.
    pub max_total_cash_minor: i64,
    pub max_installments: u8,
    /// Days of the week payments run (0 = Monday .. 6 = Sunday).
    pub payment_run_weekdays: Vec<u8>,
    /// Discount it will ask for when it counters, in basis points of the balance.
    pub discount_ask_bps: i32,
}

impl BuyerMandate {
    /// A believable mid-size client: pays on Fridays, cannot release large
    /// sums without sign-off, will ask for a small discount if it is asked to
    /// pay sooner than its own schedule.
    pub fn example(company: impl Into<String>) -> Self {
        let company = company.into();
        Self {
            agent_id: format!("ap-{}", company.to_lowercase().replace(' ', "-")),
            contact_name: "Accounts Payable".to_string(),
            company,
            policy: vec![
                "Payment runs execute on Fridays.".to_string(),
                "Purchases are matched against the engagement before release.".to_string(),
                "Anything above $1,200 in a single release needs the finance lead's approval."
                    .to_string(),
            ],
            max_unapproved_cash_minor: 120_000, // $1,200 needs no sign-off
            max_total_cash_minor: 1_000_000,
            max_installments: 3,
            payment_run_weekdays: vec![4], // Friday
            discount_ask_bps: 300,         // asks for 3 %
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuyerDecision {
    Accept {
        reason: String,
    },
    Counter {
        reason: String,
    },
    Reject {
        reason: String,
    },
    /// Terms are acceptable, but a human must approve the cash release.
    NeedsApproval {
        reason: String,
    },
}

impl BuyerDecision {
    pub fn label(&self) -> &'static str {
        match self {
            BuyerDecision::Accept { .. } => "accepted",
            BuyerDecision::Counter { .. } => "countered",
            BuyerDecision::Reject { .. } => "rejected",
            BuyerDecision::NeedsApproval { .. } => "needs approval",
        }
    }

    pub fn reason(&self) -> &str {
        match self {
            BuyerDecision::Accept { reason }
            | BuyerDecision::Counter { reason }
            | BuyerDecision::Reject { reason }
            | BuyerDecision::NeedsApproval { reason } => reason,
        }
    }
}

pub struct BuyerAgent {
    pub mandate: BuyerMandate,
    pub(crate) signing_key: SigningKey,
    llm: std::sync::Arc<dyn LlmProvider>,
}

impl BuyerAgent {
    pub fn new(mandate: BuyerMandate, llm: std::sync::Arc<dyn LlmProvider>) -> Self {
        Self {
            mandate,
            signing_key: SigningKey::generate(&mut OsRng),
            llm,
        }
    }

    /// The agent's public key, safe to publish.
    pub fn signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    pub fn agent_ref(&self) -> AgentRef {
        AgentRef {
            agent_id: self.mandate.agent_id.clone(),
            company: self.mandate.company.clone(),
            public_key: public_key_b64(&self.signing_key),
        }
    }

    pub fn mandate_json(&self) -> String {
        serde_json::json!({
            "kind": "buyer_ap_mandate",
            "agent_id": self.mandate.agent_id,
            "company": self.mandate.company,
            "policy": self.mandate.policy,
            "max_unapproved_cash_minor": self.mandate.max_unapproved_cash_minor,
            "max_total_cash_minor": self.mandate.max_total_cash_minor,
            "max_installments": self.mandate.max_installments,
            "payment_run_weekdays": self.mandate.payment_run_weekdays,
        })
        .to_string()
    }

    fn attestation(&self) -> MandateAttestation {
        MandateAttestation::sign(
            &self.signing_key,
            &self.mandate.agent_id,
            &self.mandate_json(),
        )
    }

    /// Sign and wrap a message for sending.
    pub fn envelope(&self, message: KatpMessage, include_mandate: bool) -> KatpEnvelope {
        KatpEnvelope::build(
            &self.signing_key,
            self.agent_ref(),
            include_mandate.then(|| self.attestation()),
            message,
        )
    }

    pub fn hello(&self) -> KatpEnvelope {
        self.envelope(
            KatpMessage::Hello {
                agent: self.mandate.agent_id.clone(),
                company: self.mandate.company.clone(),
                mandate_digest: self.mandate.policy.clone(),
            },
            true,
        )
    }

    /// Decide how to answer the supplier's terms.
    pub async fn respond(
        &self,
        incoming: &KatpMessage,
        balance_minor: i64,
        currency: &str,
    ) -> (KatpEnvelope, BuyerDecision) {
        let (decision, reply) = self.decide(incoming, balance_minor, currency);
        let note = self.note_for(&decision, &reply, incoming).await;
        let message = match reply {
            BuyerReply::Accept(terms) => KatpMessage::Accept { terms, note },
            BuyerReply::Counter(terms) => KatpMessage::Counter { terms, note },
            BuyerReply::Reject(reason) => KatpMessage::Reject { reason, note },
            BuyerReply::Info => KatpMessage::Info { note },
        };
        // Send the mandate attestation alongside a binding message.
        let include_mandate = matches!(
            message,
            KatpMessage::Accept { .. } | KatpMessage::Counter { .. }
        );
        (self.envelope(message, include_mandate), decision)
    }

    fn decide(
        &self,
        incoming: &KatpMessage,
        balance_minor: i64,
        currency: &str,
    ) -> (BuyerDecision, BuyerReply) {
        match incoming {
            KatpMessage::Offer { terms, .. } | KatpMessage::Counter { terms, .. } => {
                if terms.pay_now_minor <= self.mandate.max_unapproved_cash_minor {
                    (
                        BuyerDecision::Accept {
                            reason: format!(
                                "Within our ${} release limit.",
                                self.mandate.max_unapproved_cash_minor / 100
                            ),
                        },
                        BuyerReply::Accept(terms.clone()),
                    )
                } else if terms.pay_now_minor <= self.mandate.max_total_cash_minor {
                    let countered = self.counter_terms(terms, balance_minor, currency);
                    (
                        BuyerDecision::NeedsApproval {
                            reason: format!(
                                "Supplier wants ${} up front; our single-release limit is ${}.",
                                terms.pay_now_minor / 100,
                                self.mandate.max_unapproved_cash_minor / 100
                            ),
                        },
                        BuyerReply::Counter(countered),
                    )
                } else {
                    (
                        BuyerDecision::Reject {
                            reason: "Terms exceed what we can commit.".to_string(),
                        },
                        BuyerReply::Reject("terms exceed our cash policy".to_string()),
                    )
                }
            }
            KatpMessage::Settle { reference, .. } => (
                BuyerDecision::Accept {
                    reason: format!("Settlement recorded ({reference})."),
                },
                BuyerReply::Info,
            ),
            _ => (
                BuyerDecision::Accept {
                    reason: "Acknowledged.".to_string(),
                },
                BuyerReply::Info,
            ),
        }
    }

    /// Build a realistic counter: pay on our run day, within our release
    /// limit, on our payment schedule, asking a discount if we are being asked
    /// to move faster than we normally would.
    fn counter_terms(
        &self,
        incoming: &TermsProposal,
        balance_minor: i64,
        currency: &str,
    ) -> TermsProposal {
        let pay_now = self.mandate.max_unapproved_cash_minor.min(balance_minor);
        // The discount we ask for has to come out of what we pay, not on top.
        let discount_minor =
            (balance_minor as i128 * self.mandate.discount_ask_bps as i128 / 10_000) as i64;
        let remainder = (balance_minor - pay_now - discount_minor).max(0);
        let target_installments =
            self.mandate
                .max_installments
                .max(1)
                .min(if remainder > 0 { 3 } else { 0 });

        let mut installments = Vec::new();
        if target_installments > 0 && remainder > 0 {
            let per = remainder / target_installments as i64;
            let mut leftover = remainder - per * target_installments as i64;
            let today = chrono::Utc::now().date_naive();
            for idx in 0..target_installments {
                let mut amount = per;
                if leftover > 0 {
                    amount += 1;
                    leftover -= 1;
                }
                installments.push(ProposedInstallment {
                    amount_minor: amount,
                    due: next_payment_run(
                        today,
                        &self.mandate.payment_run_weekdays,
                        idx as i64 + 1,
                    ),
                });
            }
        }

        TermsProposal {
            pay_now_minor: incoming.pay_now_minor.min(pay_now),
            discount_minor,
            currency: currency.to_string(),
            installments,
            valid_until: incoming.valid_until,
        }
    }

    async fn note_for(
        &self,
        decision: &BuyerDecision,
        reply: &BuyerReply,
        incoming: &KatpMessage,
    ) -> String {
        let fallback =
            match reply {
                BuyerReply::Counter(terms) => {
                    format!(
                "{} can release ${} on the next Friday run, then ${} across {} later run{}.",
                self.mandate.company,
                terms.pay_now_minor / 100,
                terms.installments.iter().map(|i| i.amount_minor / 100).sum::<i64>(),
                terms.installments.len(),
                if terms.installments.len() == 1 { "" } else { "s" },
            )
                }
                BuyerReply::Accept(_) => format!(
                    "{} accepts — the first payment goes out with Friday's run.",
                    self.mandate.company
                ),
                BuyerReply::Reject(reason) => {
                    format!("{} can't agree to that: {reason}.", self.mandate.company)
                }
                BuyerReply::Info => "Acknowledged.".to_string(),
            };

        if !self.llm.is_enabled() {
            return fallback;
        }

        let incoming_label = incoming.label();
        let user = format!(
            "You are the accounts-payable agent for {company}.\nPolicy:\n- {policy}\n\nIncoming message: {incoming_label}.\nDecision: {decision} ({reason}).\nWrite one short sentence for the negotiation transcript, in your company's voice, no emoji.",
            company = self.mandate.company,
            policy = self.mandate.policy.join("\n- "),
            incoming_label = incoming_label,
            decision = decision.label(),
            reason = decision.reason(),
        );

        let parsed: Option<NoteShape> = complete_json(
            self.llm.as_ref(),
            LlmRequest::json(prompts::BUYER_AGENT_SYSTEM, user),
        )
        .await;
        parsed.map(|n| n.note).unwrap_or(fallback)
    }
}

#[derive(Debug, Deserialize)]
struct NoteShape {
    #[allow(dead_code)]
    decision: Option<String>,
    note: String,
}

enum BuyerReply {
    Accept(TermsProposal),
    Counter(TermsProposal),
    Reject(String),
    Info,
}

/// The next occurrence of a payment-run weekday after `from`.
fn next_payment_run(from: chrono::NaiveDate, weekdays: &[u8], run_index: i64) -> chrono::NaiveDate {
    use chrono::{Datelike, Duration};
    if weekdays.is_empty() {
        return from + Duration::days(7 * run_index);
    }
    let mut date = from;
    let mut found = 0i64;
    loop {
        date += Duration::days(1);
        let weekday = date.weekday().num_days_from_monday() as u8;
        if weekdays.contains(&weekday) {
            found += 1;
            if found >= run_index {
                return date;
            }
        }
        if (date - from).num_days() > 400 {
            return date;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::NullLlm;
    use std::sync::Arc;

    fn agent() -> BuyerAgent {
        BuyerAgent::new(BuyerMandate::example("Acme Studio"), Arc::new(NullLlm))
    }

    fn offer(pay_now_minor: i64) -> KatpMessage {
        KatpMessage::Offer {
            terms: TermsProposal {
                pay_now_minor,
                discount_minor: 0,
                currency: "USD".into(),
                installments: vec![ProposedInstallment {
                    amount_minor: 240_000 - pay_now_minor,
                    due: chrono::NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
                }],
                valid_until: chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            },
            note: "70% now".into(),
        }
    }

    #[tokio::test]
    async fn small_offers_are_accepted_directly() {
        let agent = agent();
        let (envelope, decision) = agent.respond(&offer(96_000), 240_000, "USD").await;
        assert!(matches!(decision, BuyerDecision::Accept { .. }));
        assert!(matches!(envelope.message, KatpMessage::Accept { .. }));
        assert!(envelope.verify());
        assert!(
            envelope.mandate.is_some(),
            "binding messages carry the mandate"
        );
    }

    #[tokio::test]
    async fn large_offers_are_countered_with_a_realistic_plan() {
        let agent = agent();
        let (envelope, decision) = agent.respond(&offer(168_000), 240_000, "USD").await;
        match decision {
            BuyerDecision::NeedsApproval { .. } => {}
            other => panic!("expected NeedsApproval, got {other:?}"),
        }
        match &envelope.message {
            KatpMessage::Counter { terms, .. } => {
                assert!(terms.pay_now_minor <= agent.mandate.max_unapproved_cash_minor);
                assert!(!terms.installments.is_empty());
                assert!(terms.discount_minor > 0, "AP agent asks for a discount");
                // Payment dates land on the company's payment-run weekdays.
                for installment in &terms.installments {
                    use chrono::Datelike;
                    assert_eq!(installment.due.weekday().num_days_from_monday(), 4);
                }
            }
            other => panic!("expected counter, got {other:?}"),
        }
        assert!(envelope.verify());
    }

    #[tokio::test]
    async fn absurd_offers_are_rejected() {
        let agent = agent();
        // $24,000 for a $2,400 balance: beyond anything the policy can commit.
        let (_, decision) = agent.respond(&offer(2_400_000), 240_000, "USD").await;
        assert!(matches!(decision, BuyerDecision::Reject { .. }));
    }

    #[test]
    fn hello_envelope_carries_a_verifiable_mandate() {
        let agent = agent();
        let hello = agent.hello();
        assert!(hello.verify());
        let mandate = hello.mandate.as_ref().expect("mandate present");
        assert!(mandate.verify_with_key(&hello.from.public_key, &agent.mandate_json()));
    }

    #[test]
    fn payment_runs_land_on_configured_weekdays() {
        use chrono::{Datelike, NaiveDate};
        let from = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(); // Monday
        let first = next_payment_run(from, &[4], 1);
        assert_eq!(first.weekday().num_days_from_monday(), 4);
        assert_eq!(first, NaiveDate::from_ymd_opt(2026, 10, 16).unwrap());
        let second = next_payment_run(from, &[4], 2);
        assert_eq!(second, NaiveDate::from_ymd_opt(2026, 10, 23).unwrap());
    }
}
