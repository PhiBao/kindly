//! The planner: deciding Kindly's next move on one invoice.
//!
//! Shape of the loop:
//!
//! 1. Build the candidate moves that could apply right now.
//! 2. If a model is configured, ask it to pick one and explain itself —
//!    from *that list only*. If it picks nothing valid, use the deterministic
//!    default.
//! 3. Compute exact terms with [`kindly_core::negotiation::build_terms`] so the
//!    numbers are always mandate-shaped, whatever the model asked for.
//! 4. Hand the chosen move to the caller, which runs it through the policy
//!    engine and either executes it, queues it for approval, or blocks it.
//!
//! The planner never executes anything itself.

use crate::compose::{brief_line, compose_message, ComposeFacts, CompositionSource};
use crate::judge;
use crate::llm::{complete_json, LlmProvider, LlmRequest};
use crate::prompts;
use crate::systemone::SystemOne;
use chrono::NaiveDate;
use kindly_core::action::ActionKind;
use kindly_core::client::RiskBand;
use kindly_core::mandate::{Channel, CollectionMandate, Tone};
use kindly_core::money::Currency;
use kindly_core::negotiation::{
    build_terms, EffectPurpose, ReplyIntent, Stage, Terms, TermsParams,
};
use serde::Deserialize;
use std::sync::Arc;

/// Everything the planner needs to know about one invoice.
#[derive(Debug, Clone)]
pub struct InvoiceBrief {
    pub invoice_number: String,
    pub currency: String,
    pub total_minor: i64,
    pub outstanding_minor: i64,
    pub due_date: NaiveDate,
    pub today: NaiveDate,
    pub days_overdue: i64,
    pub client_company: String,
    pub client_contact: String,
    pub client_avg_days_late: i32,
    pub client_risk: RiskBand,
    pub work_summary: Option<String>,
    pub stage: Stage,
    pub contact_count: u8,
    pub last_contact_at: Option<chrono::DateTime<chrono::Utc>>,
    pub promised_on: Option<NaiveDate>,
    /// A client reply that is waiting for a response.
    pub pending_reply: Option<ReplyIntent>,
    pub rounds: u8,
    pub client_has_ap_agent: bool,
    /// Actions already waiting for the owner on this invoice. A move that is
    /// already in the inbox must not be proposed again — otherwise every
    /// scheduler pass would stack another copy of the same question.
    pub pending_kinds: Vec<ActionKind>,
    /// A live agent-to-agent session already exists for this invoice.
    pub open_a2a_session: bool,
    pub pay_link: String,
    /// (author label, text), oldest first.
    pub history: Vec<(String, String)>,
}

impl InvoiceBrief {
    fn currency(&self) -> Currency {
        Currency::from_code(&self.currency).unwrap_or(Currency::Usd)
    }

    fn outstanding(&self) -> kindly_core::money::Money {
        kindly_core::money::Money::new(self.outstanding_minor, self.currency())
    }

    fn facts(&self) -> ComposeFacts {
        ComposeFacts {
            owner_name: String::new(), // filled in by the planner
            client_contact: self.client_contact.clone(),
            client_company: self.client_company.clone(),
            invoice_number: self.invoice_number.clone(),
            currency: self.currency.clone(),
            amount_outstanding: Some(self.outstanding()),
            total: Some(kindly_core::money::Money::new(
                self.total_minor,
                self.currency(),
            )),
            due_date: Some(self.due_date),
            days_overdue: self.days_overdue,
            work_summary: self.work_summary.clone(),
            pay_link: self.pay_link.clone(),
            plan_summary: None,
            promised_on: self.promised_on,
            history: self.history.clone(),
            language_hint: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DraftMessage {
    pub action_kind: ActionKind,
    pub channel: Channel,
    pub tone: Tone,
    pub purpose: EffectPurpose,
    pub subject: String,
    pub body: String,
    pub source: CompositionSource,
}

#[derive(Debug, Clone, Default)]
pub struct PlanOutcome {
    /// Candidate actions, in the order the planner prefers them.
    pub actions: Vec<kindly_core::policy::ProposedAction>,
    /// Draft messages bound to nudge/offer actions by index.
    pub drafts: Vec<DraftMessage>,
    /// Agent narration for the ledger ("thinking out loud", honest and factual).
    pub narrative: Vec<String>,
    /// Set when the planner deliberately does nothing.
    pub idle_reason: Option<String>,
}

pub struct Planner {
    llm: Arc<dyn LlmProvider>,
    system_one: Arc<dyn SystemOne>,
    owner_name: String,
}

/// The closed set of moves the planner may choose from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Move {
    Nudge,
    OfferPlan,
    Escalate,
    Wait,
    AgentToAgent,
    CloseUncollectible,
}

impl Move {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nudge" => Some(Move::Nudge),
            "offer_plan" => Some(Move::OfferPlan),
            "escalate" => Some(Move::Escalate),
            "wait" => Some(Move::Wait),
            "agent_to_agent" => Some(Move::AgentToAgent),
            "close_uncollectible" => Some(Move::CloseUncollectible),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Move::Nudge => "nudge",
            Move::OfferPlan => "offer_plan",
            Move::Escalate => "escalate",
            Move::Wait => "wait",
            Move::AgentToAgent => "agent_to_agent",
            Move::CloseUncollectible => "close_uncollectible",
        }
    }

    /// How the move is described to a judgment model. Concrete, and honest
    /// about what each move does.
    fn describe(self) -> &'static str {
        match self {
            Move::Nudge => {
                "Send one more polite reminder, in the owner's voice, with a payment link"
            }
            Move::OfferPlan => {
                "Put concrete payment terms on the table (an up-front amount plus instalments), inside the owner's limits"
            }
            Move::Escalate => {
                "Stop chasing and ask the owner to step in personally"
            }
            Move::Wait => {
                "Do nothing for now: the situation is already moving, or contact would be counter-productive"
            }
            Move::AgentToAgent => {
                "Open machine-to-machine talks with the client's accounts-payable agent, with signed terms"
            }
            Move::CloseUncollectible => {
                "Propose closing the receivable as uncollectible and writing it off, for the owner to decide"
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct MoveChoice {
    move_: String,
    reason: String,
    #[serde(default)]
    #[serde(rename = "move")]
    _move_alias: Option<String>,
}

impl Planner {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        system_one: Arc<dyn SystemOne>,
        owner_name: impl Into<String>,
    ) -> Self {
        Self {
            llm,
            system_one,
            owner_name: owner_name.into(),
        }
    }

    pub fn owner_name(&self) -> &str {
        &self.owner_name
    }

    /// Produce the next move for one invoice. Idempotent and pure with respect
    /// to the database: the caller decides what to do with the outcome.
    pub async fn plan(&self, brief: &InvoiceBrief, mandate: &CollectionMandate) -> PlanOutcome {
        let mut outcome = PlanOutcome::default();

        if brief.outstanding_minor <= 0 {
            outcome.idle_reason = Some("Settled — nothing outstanding.".into());
            return outcome;
        }
        if matches!(brief.stage, Stage::HumanReview) {
            outcome.idle_reason = Some("Waiting on you — this one is in human review.".into());
            return outcome;
        }
        if matches!(brief.stage, Stage::ClosedPaid | Stage::ClosedUncollectible) {
            outcome.idle_reason = Some("Closed.".into());
            return outcome;
        }

        let candidates = self.candidates(brief, mandate);

        // Ask the model to choose — from the candidate list only.
        let default_move = candidates.first().copied();
        let (chosen, why) = self
            .choose_move(brief, mandate, &candidates)
            .await
            .unwrap_or_else(|| {
                (
                    default_move.unwrap_or(Move::Wait),
                    "Default policy for this stage.".to_string(),
                )
            });
        let chosen = if candidates.contains(&chosen) {
            chosen
        } else {
            default_move.unwrap_or(Move::Wait)
        };

        match chosen {
            Move::Nudge => {
                let tone = Tone::for_contact(brief.contact_count + 1, &mandate.tone_ladder);
                let purpose = if brief.contact_count == 0 {
                    EffectPurpose::FirstNudge
                } else {
                    EffectPurpose::FollowUp
                };
                let channel = if brief.client_has_ap_agent
                    && mandate.allow_agent_to_agent
                    && brief.contact_count > 0
                {
                    Channel::AgentToAgent
                } else {
                    Channel::Portal
                };
                let mut facts = brief.facts();
                facts.owner_name = self.owner_name.clone();
                let composed = compose_message(self.llm.as_ref(), purpose, tone, &facts).await;
                outcome.drafts.push(DraftMessage {
                    action_kind: ActionKind::Nudge,
                    channel,
                    tone,
                    purpose,
                    subject: composed.subject,
                    body: composed.body,
                    source: composed.source,
                });
                outcome
                    .actions
                    .push(kindly_core::policy::ProposedAction::SendNudge { channel, tone });
                outcome.narrative.push(why);
            }
            Move::OfferPlan => {
                let params = self.terms_params(brief, mandate).await;
                match build_terms(brief.outstanding(), &params, mandate, brief.today) {
                    Ok(terms) => {
                        let tone = Tone::for_contact(brief.contact_count + 1, &mandate.tone_ladder);
                        let mut facts = brief.facts();
                        facts.owner_name = self.owner_name.clone();
                        facts.plan_summary = Some(summarise_terms(&terms, &brief.currency));
                        let composed = compose_message(
                            self.llm.as_ref(),
                            EffectPurpose::DeliveryOfPlan,
                            tone,
                            &facts,
                        )
                        .await;
                        outcome.drafts.push(DraftMessage {
                            action_kind: ActionKind::OfferTerms,
                            channel: Channel::Portal,
                            tone,
                            purpose: EffectPurpose::DeliveryOfPlan,
                            subject: composed.subject,
                            body: composed.body,
                            source: composed.source,
                        });
                        outcome
                            .actions
                            .push(kindly_core::policy::ProposedAction::OfferTerms {
                                terms,
                                channel: Channel::Portal,
                            });
                        outcome.narrative.push(why);
                    }
                    Err(error) => {
                        outcome.idle_reason =
                            Some(format!("Could not shape acceptable terms: {error}"));
                    }
                }
            }
            Move::Escalate => {
                outcome
                    .actions
                    .push(kindly_core::policy::ProposedAction::Escalate {
                        reason: format!(
                            "{} days past due with {} contact(s) and no resolution.",
                            brief.days_overdue, brief.contact_count
                        ),
                    });
                outcome.narrative.push(why);
            }
            Move::AgentToAgent => {
                let params = TermsParams {
                    upfront_bps: mandate.min_upfront_settlement_bps.max(6_000),
                    installments: 1,
                    discount_bps: 0,
                    days_between_installments: 10,
                    note: None,
                };
                match build_terms(brief.outstanding(), &params, mandate, brief.today) {
                    Ok(terms) => {
                        outcome.actions.push(
                            kindly_core::policy::ProposedAction::StartAgentNegotiation { terms },
                        );
                        outcome.narrative.push(why);
                    }
                    Err(error) => {
                        outcome.idle_reason =
                            Some(format!("Cannot open agent-to-agent talks: {error}"));
                    }
                }
            }
            Move::CloseUncollectible => {
                outcome
                    .actions
                    .push(kindly_core::policy::ProposedAction::CloseUncollectible {
                        reason: format!("No response in {} days.", brief.days_overdue),
                    });
                outcome.narrative.push(why);
            }
            Move::Wait => {
                let mut facts = brief.facts();
                facts.owner_name = self.owner_name.clone();
                let line = brief_line(self.llm.as_ref(), &facts, brief.stage.label()).await;
                outcome.idle_reason = Some(line);
                outcome.narrative.push(why);
            }
        }

        outcome
    }

    fn candidates(&self, brief: &InvoiceBrief, mandate: &CollectionMandate) -> Vec<Move> {
        let mut moves = Vec::new();

        // 1. A reply waiting for a plan is the strongest signal there is —
        // unless terms are already sitting in the owner's inbox.
        if let Some(
            ReplyIntent::RequestsPlan { .. }
            | ReplyIntent::RequestsDiscount { .. }
            | ReplyIntent::WillPaySoon { .. }
            | ReplyIntent::Unrelated { .. },
        ) = &brief.pending_reply
        {
            if !brief.pending_kinds.contains(&ActionKind::OfferTerms) {
                moves.push(Move::OfferPlan);
            }
        }

        let days_since_contact = brief
            .last_contact_at
            .map(|at| (chrono::Utc::now() - at).num_days())
            .unwrap_or(i64::MAX);

        let already_pending = |kind: ActionKind| brief.pending_kinds.contains(&kind);

        // 2. Very long silence: propose closing the receivable (owner decides).
        if brief.days_overdue >= mandate.stop_after_days_silent as i64
            && !already_pending(ActionKind::CloseUncollectible)
        {
            moves.push(Move::CloseUncollectible);
        }

        // 3. Enough contacts without resolution: stop nudging, escalate.
        if brief.contact_count >= mandate.escalate_after_contacts
            && !already_pending(ActionKind::Escalate)
        {
            moves.push(Move::Escalate);
        }

        // 4. A client with an AP agent can be negotiated with machine to machine
        //    — one live negotiation at a time.
        if brief.client_has_ap_agent
            && mandate.allow_agent_to_agent()
            && brief.stage == Stage::Idle
            && brief.days_overdue >= 5
            && !brief.open_a2a_session
        {
            moves.push(Move::AgentToAgent);
        }

        // 5. A plain reminder, when the mandate allows contact right now.
        let can_contact = brief.contact_count < mandate.escalate_after_contacts.saturating_add(3)
            && days_since_contact.max(0) >= mandate.min_days_between_contacts as i64;
        if can_contact {
            moves.push(Move::Nudge);
        }

        moves.push(Move::Wait);

        // De-duplicate while keeping order.
        let mut seen = Vec::new();
        moves.retain(|m| {
            if seen.contains(m) {
                false
            } else {
                seen.push(*m);
                true
            }
        });
        moves
    }

    async fn choose_move(
        &self,
        brief: &InvoiceBrief,
        mandate: &CollectionMandate,
        candidates: &[Move],
    ) -> Option<(Move, String)> {
        if candidates.len() <= 1 {
            return None;
        }

        // System One first: choosing one of a closed set is exactly a Choice,
        // and the answer comes with a confidence we can insist on.
        if self.system_one.is_enabled() {
            if let Some(chosen) = self
                .choose_move_with_system_one(brief, mandate, candidates)
                .await
            {
                return Some(chosen);
            }
        }

        if !self.llm.is_enabled() {
            return None;
        }
        let options = candidates
            .iter()
            .map(|m| m.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let user = format!(
            r#"Invoice {number}: {outstanding} outstanding, due {due}, {days} days past due.
Client: {company} (average {avg_late} days late, band: {risk:?}).
Conversation stage: {stage:?}. Contacts so far: {contacts}.
Last client message: {last}
Owner's limits: max {max_contacts} contacts/week, floor {floor}% up front, up to {installments} installments, discount ceiling {discount}%.

Choose exactly one next move from this list: {options}.
Consider: the client relationship, how overdue it is, and what got a response before.
Return strict JSON: {{"move": "<one of the list>", "reason": "<one sentence, factual>"}}"#,
            number = brief.invoice_number,
            outstanding = brief.outstanding().display_with_symbol(),
            due = brief.due_date,
            days = brief.days_overdue,
            company = brief.client_company,
            avg_late = brief.client_avg_days_late,
            risk = brief.client_risk,
            stage = brief.stage,
            contacts = brief.contact_count,
            last = brief
                .history
                .last()
                .map(|(_, text)| text.as_str())
                .unwrap_or("(none)"),
            max_contacts = mandate.max_contacts_per_week,
            floor = mandate.min_upfront_settlement_bps / 100,
            installments = mandate.max_installments,
            discount = mandate.max_discount_bps / 100,
            options = options,
        );

        let choice: MoveChoice = complete_json(
            self.llm.as_ref(),
            LlmRequest::json(prompts::NEGOTIATION_SYSTEM, user),
        )
        .await?;
        let parsed = Move::parse(&choice.move_)?;
        Some((parsed, choice.reason))
    }

    /// Ask TypeSafe which move to make. The candidate descriptions carry the
    /// situation; the answer carries a confidence; code keeps the verdict.
    async fn choose_move_with_system_one(
        &self,
        brief: &InvoiceBrief,
        mandate: &CollectionMandate,
        candidates: &[Move],
    ) -> Option<(Move, String)> {
        let descriptions: Vec<(&str, &str)> = candidates
            .iter()
            .map(|candidate| (candidate.as_str(), candidate.describe()))
            .collect();

        let state = serde_json::json!({
            "today": brief.today.to_string(),
            "invoice": {
                "number": brief.invoice_number,
                "outstanding": brief.outstanding().display_with_symbol(),
                "due_date": brief.due_date.to_string(),
                "days_overdue": brief.days_overdue,
                "stage": format!("{:?}", brief.stage),
                "contacts_so_far": brief.contact_count,
                "last_contact_at": brief.last_contact_at.map(|at| at.to_rfc3339()),
                "promised_on": brief.promised_on.map(|day| day.to_string()),
            },
            "client": {
                "company": brief.client_company,
                "average_days_late": brief.client_avg_days_late,
                "behaviour": format!("{:?}", brief.client_risk),
                "has_accounts_payable_agent": brief.client_has_ap_agent,
            },
            "client_last_message_reading": brief
                .pending_reply
                .as_ref()
                .map(|reply| format!("{reply:?}")),
            "recent_correspondence": brief
                .history
                .iter()
                .rev()
                .take(4)
                .rev()
                .map(|(author, text)| serde_json::json!({ "from": author, "text": text }))
                .collect::<Vec<_>>(),
            "owner_limits": {
                "max_contacts_per_week": mandate.max_contacts_per_week,
                "min_days_between_contacts": mandate.min_days_between_contacts,
                "min_upfront_settlement_percent": mandate.min_upfront_settlement_bps / 100,
                "max_installments": mandate.max_installments,
                "may_negotiate_agent_to_agent": mandate.allow_agent_to_agent,
            },
        });

        let questions = judge::move_questions(&descriptions);
        let judgment = match self.system_one.ask(state, questions).await {
            Ok(judgment) => judgment,
            Err(error) => {
                tracing::warn!(%error, "system one move judgment failed; falling back");
                return None;
            }
        };

        let (label, confidence) = judgment.choice("move")?;
        if confidence < judge::INTENT_CONFIDENCE_FLOOR {
            return None;
        }
        let chosen = Move::parse(label)?;
        if !candidates.contains(&chosen) {
            return None;
        }

        let wait = judgment.noul("should_wait").unwrap_or(0.0);
        let mut reason = format!(
            "{} chose “{}” (confidence {:.2})",
            self.system_one.description(),
            chosen.as_str(),
            confidence
        );
        if chosen == Move::Nudge && wait >= 0.6 {
            reason.push_str(&format!(
                "; it also flagged that another contact now risks the relationship ({wait:.2}), so this is the last cheap reminder"
            ));
        }
        Some((chosen, reason))
    }

    async fn terms_params(&self, brief: &InvoiceBrief, mandate: &CollectionMandate) -> TermsParams {
        // Deterministic defaults first.
        let mut params = match &brief.pending_reply {
            Some(ReplyIntent::RequestsPlan {
                pay_now,
                installments,
            }) => {
                let upfront_bps = pay_now
                    .map(|amount| {
                        if brief.outstanding_minor > 0 {
                            ((amount.minor as i128 * 10_000) / brief.outstanding_minor as i128)
                                .clamp(0, 10_000) as i32
                        } else {
                            10_000
                        }
                    })
                    .unwrap_or(mandate.min_upfront_settlement_bps);
                TermsParams {
                    upfront_bps,
                    installments: installments.unwrap_or_else(|| default_installments(brief)),
                    discount_bps: 0,
                    days_between_installments: 7,
                    note: None,
                }
            }
            Some(ReplyIntent::RequestsDiscount { bps }) => TermsParams {
                upfront_bps: mandate.min_upfront_settlement_bps,
                installments: default_installments(brief),
                discount_bps: *bps,
                days_between_installments: 7,
                note: None,
            },
            _ => TermsParams {
                upfront_bps: match brief.client_risk {
                    RiskBand::Slow => 10_000,
                    RiskBand::Watch => mandate.min_upfront_settlement_bps.max(7_500),
                    _ => mandate.min_upfront_settlement_bps,
                },
                installments: default_installments(brief),
                discount_bps: 0,
                days_between_installments: 7,
                note: None,
            },
        };

        // Let the model refine the shape within the same clamps.
        if self.llm.is_enabled() && brief.rounds < 3 {
            if let Some(refined) = self.model_terms(brief, mandate, &params).await {
                params = refined;
            }
        }
        params
    }

    async fn model_terms(
        &self,
        brief: &InvoiceBrief,
        mandate: &CollectionMandate,
        defaults: &TermsParams,
    ) -> Option<TermsParams> {
        let user = format!(
            r#"Negotiate terms for invoice {number}.
Balance: {balance}. Days past due: {days}.
Client: {company}, average {avg} days late, band {risk:?}.
They asked: {ask}
Owner limits: at least {floor}% up front; max {inst} installments within {plan_days} days; discount ceiling {disc}%; installments at least {min_inst}.
A conventional starting shape would be: {upfront}% up front, {default_inst} installments, {default_disc}% discount.

Return strict JSON: {{"upfront_bps": int, "installments": int, "discount_bps": int, "days_between_installments": int, "note": "one sentence for the ledger"}}"#,
            number = brief.invoice_number,
            balance = brief.outstanding().display_with_symbol(),
            days = brief.days_overdue,
            company = brief.client_company,
            avg = brief.client_avg_days_late,
            risk = brief.client_risk,
            ask = brief
                .pending_reply
                .as_ref()
                .map(|r| format!("{r:?}"))
                .unwrap_or_else(|| "nothing specific".into()),
            floor = mandate.min_upfront_settlement_bps / 100,
            inst = mandate.max_installments,
            plan_days = mandate.max_plan_days,
            disc = mandate.max_discount_bps / 100,
            min_inst = mandate.min_installment_amount.display_with_symbol(),
            upfront = defaults.upfront_bps / 100,
            default_inst = defaults.installments,
            default_disc = defaults.discount_bps as f64 / 100.0,
        );

        #[derive(Deserialize)]
        struct TermsShape {
            upfront_bps: i32,
            installments: u8,
            discount_bps: i32,
            days_between_installments: u16,
            #[serde(default)]
            note: Option<String>,
        }

        let shape: TermsShape = complete_json(
            self.llm.as_ref(),
            LlmRequest::json(prompts::NEGOTIATION_SYSTEM, user),
        )
        .await?;

        Some(TermsParams {
            upfront_bps: shape.upfront_bps.clamp(0, 10_000),
            installments: shape.installments.min(6),
            discount_bps: shape.discount_bps.clamp(0, 10_000),
            days_between_installments: shape.days_between_installments.clamp(1, 60),
            note: shape.note,
        })
    }
}

fn default_installments(brief: &InvoiceBrief) -> u8 {
    match brief.client_risk {
        RiskBand::Slow => 2,
        RiskBand::Watch => 1,
        RiskBand::Reliable | RiskBand::Unknown => 1,
    }
}

/// A one-line, human summary of agreed or offered terms.
pub fn summarise_terms(terms: &Terms, currency_code: &str) -> String {
    let currency = Currency::from_code(currency_code).unwrap_or(Currency::Usd);
    let mut parts = vec![format!("{} now", terms.pay_now.display_with_symbol())];
    for (idx, installment) in terms.installments.iter().enumerate() {
        let _ = currency;
        parts.push(format!(
            "{} on {}",
            installment.amount.display_with_symbol(),
            installment.due.format("%-d %B")
        ));
        if idx >= 3 {
            break;
        }
    }
    if terms.discount.is_positive() {
        parts.push(format!("{} discount", terms.discount.display_with_symbol()));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::NullLlm;
    use kindly_core::mandate::AutonomyLevel;

    fn brief() -> InvoiceBrief {
        InvoiceBrief {
            invoice_number: "1042".into(),
            currency: "USD".into(),
            total_minor: 240_000,
            outstanding_minor: 240_000,
            due_date: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            days_overdue: 11,
            client_company: "Acme Studio".into(),
            client_contact: "Sam Chen".into(),
            client_avg_days_late: 14,
            client_risk: RiskBand::Watch,
            work_summary: Some("the brand identity sprint".into()),
            stage: Stage::Idle,
            contact_count: 0,
            last_contact_at: None,
            promised_on: None,
            pending_reply: None,
            rounds: 0,
            client_has_ap_agent: true,
            pending_kinds: Vec::new(),
            open_a2a_session: false,
            pay_link: "https://kindly.demo/c/tok".into(),
            history: vec![],
        }
    }

    fn mandate() -> CollectionMandate {
        CollectionMandate::default()
    }

    #[tokio::test]
    async fn first_move_on_an_overdue_invoice_is_a_nudge() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya Rodriguez",
        );
        let mut b = brief();
        b.client_has_ap_agent = false;
        let outcome = planner.plan(&b, &mandate()).await;
        assert_eq!(outcome.actions.len(), 1);
        assert!(matches!(
            outcome.actions[0],
            kindly_core::policy::ProposedAction::SendNudge { .. }
        ));
        assert_eq!(outcome.drafts.len(), 1);
        assert!(outcome.drafts[0].body.contains("Maya"));
        assert_eq!(outcome.drafts[0].source, CompositionSource::Template);
    }

    #[tokio::test]
    async fn a_client_with_an_ap_agent_is_negotiated_with_first() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya Rodriguez",
        );
        let outcome = planner.plan(&brief(), &mandate()).await;
        assert!(matches!(
            outcome.actions[0],
            kindly_core::policy::ProposedAction::StartAgentNegotiation { .. }
        ));
    }

    #[tokio::test]
    async fn a_plan_request_produces_terms_that_cover_the_balance() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya Rodriguez",
        );
        let mut b = brief();
        b.stage = Stage::Negotiating;
        b.contact_count = 1;
        b.pending_reply = Some(ReplyIntent::RequestsPlan {
            pay_now: Some(kindly_core::money::Money::new(96_000, Currency::Usd)),
            installments: Some(2),
        });
        let outcome = planner.plan(&b, &mandate()).await;
        match &outcome.actions[0] {
            kindly_core::policy::ProposedAction::OfferTerms { terms, .. } => {
                assert!(terms.covers(b.outstanding()).unwrap());
                assert!(
                    terms.upfront_bps(b.outstanding()).unwrap()
                        >= mandate().min_upfront_settlement_bps
                );
                assert!(terms.installment_count() <= mandate().max_installments);
            }
            other => panic!("expected an offer, got {other:?}"),
        }
        assert!(
            outcome.drafts[0].body.contains("plan") || outcome.drafts[0].body.contains("agreed")
        );
    }

    #[tokio::test]
    async fn long_silence_escalates_then_proposes_closure() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya",
        );
        let mut b = brief();
        b.client_has_ap_agent = false;
        b.contact_count = 5; // beyond the escalation threshold
        b.last_contact_at = Some(chrono::Utc::now() - chrono::Duration::days(30));
        let outcome = planner.plan(&b, &mandate()).await;
        assert!(outcome
            .actions
            .iter()
            .any(|a| matches!(a, kindly_core::policy::ProposedAction::Escalate { .. })));

        let mut silent = brief();
        silent.client_has_ap_agent = false;
        silent.days_overdue = 200;
        silent.contact_count = 6;
        silent.last_contact_at = Some(chrono::Utc::now() - chrono::Duration::days(60));
        let outcome = planner.plan(&silent, &mandate()).await;
        assert!(outcome.actions.iter().any(|a| matches!(
            a,
            kindly_core::policy::ProposedAction::CloseUncollectible { .. }
        )));
    }

    #[tokio::test]
    async fn observe_only_still_plans_but_policy_will_block() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya",
        );
        let mut m = mandate();
        m.autonomy = AutonomyLevel::Observe;
        let outcome = planner.plan(&brief(), &m).await;
        // Planning is allowed; the policy engine is the gate.
        assert!(!outcome.actions.is_empty());
    }

    #[tokio::test]
    async fn settled_invoices_produce_no_actions() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya",
        );
        let mut b = brief();
        b.outstanding_minor = 0;
        let outcome = planner.plan(&b, &mandate()).await;
        assert!(outcome.actions.is_empty());
        assert!(outcome.idle_reason.is_some());
    }

    /// A judgment provider that always answers, so the choice path is exercised
    /// without a network call.
    struct ScriptedOne {
        move_label: &'static str,
        confidence: f64,
        should_wait: f64,
    }

    #[async_trait::async_trait]
    impl crate::systemone::SystemOne for ScriptedOne {
        async fn ask(
            &self,
            _state: serde_json::Value,
            _questions: std::collections::BTreeMap<String, crate::systemone::Question>,
        ) -> Result<crate::systemone::Judgment, crate::systemone::SystemOneError> {
            let mut answers = std::collections::BTreeMap::new();
            answers.insert(
                "move".to_string(),
                crate::systemone::Answer::Choice {
                    choice: self.move_label.to_string(),
                    probabilities: std::collections::BTreeMap::from([(
                        self.move_label.to_string(),
                        self.confidence,
                    )]),
                    confidence: self.confidence,
                },
            );
            answers.insert(
                "should_wait".to_string(),
                crate::systemone::Answer::Noul {
                    noul: self.should_wait,
                },
            );
            Ok(crate::systemone::Judgment {
                answers,
                model: "jev-test".into(),
                input_tokens: 120,
                output_tokens: 12,
            })
        }

        fn description(&self) -> String {
            "scripted judgment".into()
        }
    }

    #[tokio::test]
    async fn a_system_one_judgment_chooses_the_move() {
        // The deterministic default for this brief would be agent-to-agent
        // (Acme has an AP agent); the judgment says to send a reminder instead,
        // and code follows it because the candidate list allows it.
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(ScriptedOne {
                move_label: "nudge",
                confidence: 0.77,
                should_wait: 0.0,
            }),
            "Maya Rodriguez",
        );
        let outcome = planner.plan(&brief(), &mandate()).await;
        assert!(matches!(
            outcome.actions[0],
            kindly_core::policy::ProposedAction::SendNudge { .. }
        ));
        assert!(
            outcome
                .narrative
                .iter()
                .any(|line| line.contains("scripted judgment")),
            "the ledger explains who chose: {:?}",
            outcome.narrative
        );
    }

    #[tokio::test]
    async fn a_low_confidence_judgment_falls_back_to_the_default() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(ScriptedOne {
                move_label: "wait",
                confidence: 0.21,
                should_wait: 0.9,
            }),
            "Maya Rodriguez",
        );
        let outcome = planner.plan(&brief(), &mandate()).await;
        assert!(
            matches!(
                outcome.actions[0],
                kindly_core::policy::ProposedAction::StartAgentNegotiation { .. }
            ),
            "the deterministic default stands"
        );
    }

    #[tokio::test]
    async fn a_judgment_outside_the_candidate_set_is_ignored() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(ScriptedOne {
                move_label: "close_uncollectible",
                confidence: 0.99,
                should_wait: 0.0,
            }),
            "Maya Rodriguez",
        );
        // The brief is only 11 days overdue, so closure is not a candidate.
        let outcome = planner.plan(&brief(), &mandate()).await;
        assert!(
            matches!(
                outcome.actions[0],
                kindly_core::policy::ProposedAction::StartAgentNegotiation { .. }
            ),
            "an out-of-set choice cannot be acted on"
        );
    }

    #[tokio::test]
    async fn a_move_already_waiting_for_the_owner_is_not_proposed_again() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya Rodriguez",
        );
        let mut silent = brief();
        silent.client_has_ap_agent = false;
        silent.days_overdue = 200;
        silent.contact_count = 6;
        silent.last_contact_at = Some(chrono::Utc::now() - chrono::Duration::days(60));

        let first = planner.plan(&silent, &mandate()).await;
        assert!(first.actions.iter().any(|action| matches!(
            action,
            kindly_core::policy::ProposedAction::CloseUncollectible { .. }
        )));

        // The same situation, but the closure is already in the inbox.
        silent.pending_kinds = vec![ActionKind::CloseUncollectible];
        let second = planner.plan(&silent, &mandate()).await;
        assert!(
            !second.actions.iter().any(|action| matches!(
                action,
                kindly_core::policy::ProposedAction::CloseUncollectible { .. }
            )),
            "the owner is not asked the same question twice"
        );
    }

    #[tokio::test]
    async fn a_live_negotiation_is_not_opened_twice() {
        let planner = Planner::new(
            Arc::new(NullLlm),
            Arc::new(crate::systemone::NoSystemOne),
            "Maya Rodriguez",
        );
        let mut brief_with_session = brief();
        brief_with_session.open_a2a_session = true;
        brief_with_session.client_has_ap_agent = true;
        let outcome = planner.plan(&brief_with_session, &mandate()).await;
        assert!(
            !outcome.actions.iter().any(|action| matches!(
                action,
                kindly_core::policy::ProposedAction::StartAgentNegotiation { .. }
            )),
            "a second negotiation would confuse both sides"
        );
    }

    #[test]
    fn summary_is_human_readable() {
        let terms = Terms {
            pay_now: kindly_core::money::Money::new(120_000, Currency::Usd),
            discount: kindly_core::money::Money::new(5_000, Currency::Usd),
            installments: vec![kindly_core::negotiation::Installment {
                amount: kindly_core::money::Money::new(115_000, Currency::Usd),
                due: NaiveDate::from_ymd_opt(2026, 10, 24).unwrap(),
            }],
            deadline: None,
        };
        let summary = summarise_terms(&terms, "USD");
        assert!(summary.contains("$1,200.00 now"));
        assert!(summary.contains("24 October"));
        assert!(summary.contains("discount"));
    }
}
