//! The policy engine.
//!
//! This is the single place where Kindly decides whether the agent may act.
//! It is deterministic, exhaustively unit-tested, and has no knowledge of
//! prompts, models or vendors. A language model may propose; this module
//! disposes.
//!
//! Rule order is intentional:
//!   1. hard invalidity (closed invoice, nothing outstanding, already agreed)
//!   2. hard limits from the mandate (contact caps, quiet hours, discounts,
//!      settlement floor, plan shape) — these *block*
//!   3. soft limits — these require the owner's approval
//!   4. autonomy level — decides whether a valid action needs approval anyway

use crate::action::ActionKind;
use crate::invoice::{Invoice, InvoiceStatus};
use crate::mandate::{AutonomyLevel, Channel, CollectionMandate, Tone};
use crate::money::Money;
use crate::negotiation::Terms;
use crate::{Day, Timestamp};
use serde::{Deserialize, Serialize};

/// A compact view of an already-executed action, for policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSummary {
    pub kind: ActionKind,
    pub at: Timestamp,
}

impl ActionSummary {
    pub fn new(kind: ActionKind, at: Timestamp) -> Self {
        Self { kind, at }
    }
}

/// Everything the engine needs to judge one proposal.
#[derive(Debug, Clone)]
pub struct PolicyContext<'a> {
    pub now: Timestamp,
    pub mandate: &'a CollectionMandate,
    pub invoice: &'a Invoice,
    /// Actions executed on this invoice, newest first or not — order is ignored.
    pub recent_actions: &'a [ActionSummary],
    /// True when an escalation is already awaiting the owner.
    pub open_escalation: bool,
}

impl<'a> PolicyContext<'a> {
    fn contacts_last_7_days(&self) -> u8 {
        let cutoff = self.now - chrono::Duration::days(7);
        self.recent_actions
            .iter()
            .filter(|a| a.kind.is_client_contact() && a.at >= cutoff)
            .count()
            .min(u8::MAX as usize) as u8
    }

    fn last_contact_at(&self) -> Option<Timestamp> {
        self.recent_actions
            .iter()
            .filter(|a| a.kind.is_client_contact())
            .map(|a| a.at)
            .max()
    }

    fn days_since_last_contact(&self) -> Option<i64> {
        self.last_contact_at().map(|at| (self.now - at).num_days())
    }
}

/// What the agent would like to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProposedAction {
    SendNudge {
        channel: Channel,
        tone: Tone,
    },
    OfferTerms {
        terms: Terms,
        channel: Channel,
    },
    IssuePlan {
        terms: Terms,
    },
    RecordPartialPayment {
        amount: Money,
    },
    GrantDiscount {
        amount: Money,
        reason: String,
    },
    Escalate {
        reason: String,
    },
    Pause {
        days: u16,
        reason: String,
    },
    StartAgentNegotiation {
        terms: Terms,
    },
    CloseUncollectible {
        reason: String,
    },
}

impl ProposedAction {
    pub fn kind(&self) -> ActionKind {
        match self {
            ProposedAction::SendNudge { .. } => ActionKind::Nudge,
            ProposedAction::OfferTerms { .. } => ActionKind::OfferTerms,
            ProposedAction::IssuePlan { .. } => ActionKind::IssuePlan,
            ProposedAction::RecordPartialPayment { .. } => ActionKind::RecordPartialPayment,
            ProposedAction::GrantDiscount { .. } => ActionKind::GrantDiscount,
            ProposedAction::Escalate { .. } => ActionKind::Escalate,
            ProposedAction::Pause { .. } => ActionKind::Pause,
            ProposedAction::StartAgentNegotiation { .. } => ActionKind::StartAgentNegotiation,
            ProposedAction::CloseUncollectible { .. } => ActionKind::CloseUncollectible,
        }
    }

    /// Does this proposal put a message in front of the client?
    pub fn contacts_client(&self) -> bool {
        self.kind().is_client_contact()
    }

    pub fn channel(&self) -> Option<Channel> {
        match self {
            ProposedAction::SendNudge { channel, .. } => Some(*channel),
            ProposedAction::OfferTerms { channel, .. } => Some(*channel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PolicyDecision {
    Allow,
    RequireApproval { reason: ApprovalReason },
    Block { reason: BlockReason },
}

impl PolicyDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, PolicyDecision::Block { .. })
    }

    /// Short, human sentence for the UI and the audit log.
    pub fn explain(&self) -> String {
        match self {
            PolicyDecision::Allow => "Within your limits".to_string(),
            PolicyDecision::RequireApproval { reason } => reason.explain().to_string(),
            PolicyDecision::Block { reason } => reason.explain().to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ApprovalReason {
    /// Autonomy is set to "suggest only".
    AutonomySuggestOnly,
    /// A discount above the pre-approved limit.
    DiscountAboveAutoLimit { requested_bps: i32, auto_limit_bps: i32 },
    /// Terms that would take longer than the mandate allows.
    PlanAboveAutoLimit { days: i64, max_days: u16 },
    /// Escalation that needs the owner's blessing.
    Escalation,
    /// Agent-to-agent is on, but this particular proposal is new territory.
    AgentToAgentFirstContact,
}

impl ApprovalReason {
    pub fn explain(&self) -> String {
        match self {
            ApprovalReason::AutonomySuggestOnly => {
                "Your mandate is set to “suggest, I approve”, so nothing goes out without you.".into()
            }
            ApprovalReason::DiscountAboveAutoLimit { requested_bps, auto_limit_bps } => format!(
                "This grants a {}% discount — above the {}% you pre-approved.",
                fmt_bps(*requested_bps),
                fmt_bps(*auto_limit_bps)
            ),
            ApprovalReason::PlanAboveAutoLimit { days, max_days } => format!(
                "This plan runs {days} days, longer than the {max_days} days you pre-approved."
            ),
            ApprovalReason::Escalation => "Escalating needs your sign-off.".into(),
            ApprovalReason::AgentToAgentFirstContact => {
                "First agent-to-agent contact with this client.".into()
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum BlockReason {
    AutonomyObserve,
    InvoiceNotOpen { status: String },
    NothingOutstanding,
    AlreadySettled,
    WeeklyContactCap { used: u8, cap: u8 },
    TooSoon { days_since: i64, min_days: u8 },
    QuietHours { window: String },
    DiscountAboveCeiling { requested_bps: i32, ceiling_bps: i32 },
    BelowSettlementFloor { upfront_bps: i32, floor_bps: i32 },
    TooManyInstallments { requested: u8, max: u8 },
    InstallmentTooSmall { amount: String, min: String },
    PlanLongerThanAllowed { days: i64, max_days: u16 },
    TermsDoNotCover { total: String, outstanding: String },
    AgentToAgentDisabled,
    EscalationAlreadyOpen,
}

impl BlockReason {
    pub fn explain(&self) -> String {
        match self {
            BlockReason::AutonomyObserve => {
                "Your mandate is “observe only”, so Kindly does not contact anyone.".into()
            }
            BlockReason::InvoiceNotOpen { status } => {
                format!("Invoice is {status} — nothing to collect.")
            }
            BlockReason::NothingOutstanding => "Nothing is outstanding on this invoice.".into(),
            BlockReason::AlreadySettled => "Terms are already agreed for this invoice.".into(),
            BlockReason::WeeklyContactCap { used, cap } => format!(
                "Blocked: weekly contact cap reached ({used} of {cap})."
            ),
            BlockReason::TooSoon { days_since, min_days } => format!(
                "Blocked: only {days_since} day(s) since the last contact (you asked for at least {min_days})."
            ),
            BlockReason::QuietHours { window } => format!(
                "Blocked by quiet hours — clients may only be contacted {window}."
            ),
            BlockReason::DiscountAboveCeiling { requested_bps, ceiling_bps } => format!(
                "Blocked: {}% discount exceeds your {}% ceiling.",
                fmt_bps(*requested_bps),
                fmt_bps(*ceiling_bps)
            ),
            BlockReason::BelowSettlementFloor { upfront_bps, floor_bps } => format!(
                "Blocked: only {}% paid up front — below your {}% floor.",
                fmt_bps(*upfront_bps),
                fmt_bps(*floor_bps)
            ),
            BlockReason::TooManyInstallments { requested, max } => format!(
                "Blocked: {requested} installments exceeds your limit of {max}."
            ),
            BlockReason::InstallmentTooSmall { amount, min } => format!(
                "Blocked: an installment of {amount} is below your {min} minimum."
            ),
            BlockReason::PlanLongerThanAllowed { days, max_days } => format!(
                "Blocked: the plan runs {days} days — your limit is {max_days}."
            ),
            BlockReason::TermsDoNotCover { total, outstanding } => format!(
                "Blocked: terms total {total} against {outstanding} outstanding."
            ),
            BlockReason::AgentToAgentDisabled => {
                "Blocked: agent-to-agent negotiation is switched off in your mandate.".into()
            }
            BlockReason::EscalationAlreadyOpen => {
                "Blocked: an escalation is already waiting for you.".into()
            }
        }
    }
}

fn fmt_bps(bps: i32) -> String {
    let whole = bps / 100;
    let frac = (bps % 100).abs();
    if frac == 0 {
        format!("{whole}")
    } else {
        format!("{whole}.{frac:02}")
    }
}

/// Judge a proposal. Pure function: same inputs, same verdict, no I/O.
pub fn evaluate(proposal: &ProposedAction, ctx: &PolicyContext<'_>) -> PolicyDecision {
    let mandate = ctx.mandate;
    let invoice = ctx.invoice;

    // 1 ─ hard invalidity -------------------------------------------------
    if matches!(mandate.autonomy, AutonomyLevel::Observe) {
        return blocked(BlockReason::AutonomyObserve);
    }
    if invoice.status == InvoiceStatus::Paid {
        return blocked(BlockReason::AlreadySettled);
    }
    if !invoice.status.is_open() && !matches!(proposal, ProposedAction::Pause { .. }) {
        return blocked(BlockReason::InvoiceNotOpen {
            status: invoice.status.label().to_lowercase(),
        });
    }
    let outstanding = invoice.outstanding();
    if outstanding.minor <= 0 && !matches!(proposal, ProposedAction::Pause { .. } | ProposedAction::CloseUncollectible { .. }) {
        return blocked(BlockReason::NothingOutstanding);
    }

    // 2 ─ hard mandate limits --------------------------------------------
    if proposal.contacts_client() {
        // Quiet hours apply to human-facing channels. AP systems are machines
        // and are exempt: the counterparty agent is expected to be async.
        if let Some(channel) = proposal.channel() {
            if channel != Channel::AgentToAgent && !mandate.contact_window.allows(ctx.now) {
                return blocked(BlockReason::QuietHours {
                    window: mandate.contact_window.describe(),
                });
            }
        }
        let used = ctx.contacts_last_7_days();
        if used >= mandate.max_contacts_per_week {
            return blocked(BlockReason::WeeklyContactCap {
                used,
                cap: mandate.max_contacts_per_week,
            });
        }
        if let Some(days_since) = ctx.days_since_last_contact() {
            if days_since < mandate.min_days_between_contacts as i64 {
                return blocked(BlockReason::TooSoon {
                    days_since,
                    min_days: mandate.min_days_between_contacts,
                });
            }
        }
    }

    match proposal {
        ProposedAction::SendNudge { .. } => {}
        ProposedAction::OfferTerms { terms, .. }
        | ProposedAction::IssuePlan { terms }
        | ProposedAction::StartAgentNegotiation { terms } => {
            if let ProposedAction::StartAgentNegotiation { .. } = proposal {
                if !mandate.allow_agent_to_agent() {
                    return blocked(BlockReason::AgentToAgentDisabled);
                }
            }
            if let Some(reason) = judge_terms(terms, outstanding, mandate, ctx.today()) {
                return reason;
            }
        }
        ProposedAction::GrantDiscount { amount, .. } => {
            if amount.currency != invoice.currency {
                return blocked(BlockReason::TermsDoNotCover {
                    total: amount.to_string(),
                    outstanding: outstanding.to_string(),
                });
            }
            let requested_bps = if outstanding.minor > 0 {
                ((amount.minor as i128 * 10_000) / outstanding.minor as i128) as i32
            } else {
                0
            };
            if requested_bps > mandate.max_discount_bps {
                return blocked(BlockReason::DiscountAboveCeiling {
                    requested_bps,
                    ceiling_bps: mandate.max_discount_bps,
                });
            }
            if requested_bps > mandate.max_discount_without_approval_bps {
                return PolicyDecision::RequireApproval {
                    reason: ApprovalReason::DiscountAboveAutoLimit {
                        requested_bps,
                        auto_limit_bps: mandate.max_discount_without_approval_bps,
                    },
                };
            }
        }
        ProposedAction::RecordPartialPayment { amount } => {
            if amount.is_negative() || amount.is_zero() {
                return blocked(BlockReason::NothingOutstanding);
            }
        }
        ProposedAction::Escalate { .. } => {
            if ctx.open_escalation {
                return blocked(BlockReason::EscalationAlreadyOpen);
            }
        }
        ProposedAction::Pause { .. } => {}
        ProposedAction::CloseUncollectible { .. } => {
            // Closing a receivable is always a human decision.
            return PolicyDecision::RequireApproval {
                reason: ApprovalReason::Escalation,
            };
        }
    }

    // 3 ─ autonomy -------------------------------------------------------
    match mandate.autonomy {
        AutonomyLevel::Observe => blocked(BlockReason::AutonomyObserve),
        AutonomyLevel::SuggestOnly => PolicyDecision::RequireApproval {
            reason: ApprovalReason::AutonomySuggestOnly,
        },
        AutonomyLevel::ActWithinLimits => {
            if let ProposedAction::Escalate { .. } = proposal {
                if !ctx.open_escalation && !proposal.contacts_client() {
                    // Escalations are inbound to the owner; allow automatically
                    // only when the mandate's own contact ladder triggered it.
                    return PolicyDecision::RequireApproval {
                        reason: ApprovalReason::Escalation,
                    };
                }
            }
            PolicyDecision::Allow
        }
    }
}

impl<'a> PolicyContext<'a> {
    fn today(&self) -> Day {
        self.now.date_naive()
    }
}

/// Judge a proposed set of terms against the mandate's money rules only.
///
/// Used by the agent-to-agent path (where contact caps and quiet hours do not
/// apply, because the counterparty is a machine).
pub fn evaluate_terms(
    terms: &Terms,
    invoice: &Invoice,
    mandate: &CollectionMandate,
    today: Day,
) -> PolicyDecision {
    match judge_terms(terms, invoice.outstanding(), mandate, today) {
        Some(decision) => decision,
        None => PolicyDecision::Allow,
    }
}

pub fn judge_terms(
    terms: &Terms,
    outstanding: Money,
    mandate: &CollectionMandate,
    today: Day,
) -> Option<PolicyDecision> {
    // Coverage first: terms must actually clear the debt.
    match terms.covers(outstanding) {
        Ok(true) => {}
        _ => {
            return Some(blocked(BlockReason::TermsDoNotCover {
                total: terms
                    .total_value()
                    .map(|m| m.to_string())
                    .unwrap_or_else(|_| "invalid".into()),
                outstanding: outstanding.to_string(),
            }))
        }
    }

    if let Ok(total) = terms.total_value() {
        if total.minor > 0 {
            let discount_bps = ((terms.discount.minor as i128 * 10_000) / total.minor as i128) as i32;
            if discount_bps > mandate.max_discount_bps {
                return Some(blocked(BlockReason::DiscountAboveCeiling {
                    requested_bps: discount_bps,
                    ceiling_bps: mandate.max_discount_bps,
                }));
            }
        }
    }

    let upfront = terms.upfront_bps(outstanding).unwrap_or(10_000);
    if upfront < mandate.min_upfront_settlement_bps {
        return Some(blocked(BlockReason::BelowSettlementFloor {
            upfront_bps: upfront,
            floor_bps: mandate.min_upfront_settlement_bps,
        }));
    }

    let installments = terms.installment_count();
    if installments > mandate.max_installments {
        return Some(blocked(BlockReason::TooManyInstallments {
            requested: installments,
            max: mandate.max_installments,
        }));
    }

    if let Some(smallest) = terms.smallest_installment() {
        if smallest.minor < mandate.min_installment_amount.minor {
            return Some(blocked(BlockReason::InstallmentTooSmall {
                amount: smallest.to_string(),
                min: mandate.min_installment_amount.to_string(),
            }));
        }
    }

    let days = terms.days_to_complete(today);
    if days > mandate.max_plan_days as i64 {
        return Some(blocked(BlockReason::PlanLongerThanAllowed {
            days,
            max_days: mandate.max_plan_days,
        }));
    }

    // Soft limits → approval.
    let discount_bps = if let Ok(total) = terms.total_value() {
        if total.minor > 0 {
            ((terms.discount.minor as i128 * 10_000) / total.minor as i128) as i32
        } else {
            0
        }
    } else {
        0
    };
    if discount_bps > mandate.max_discount_without_approval_bps {
        return Some(PolicyDecision::RequireApproval {
            reason: ApprovalReason::DiscountAboveAutoLimit {
                requested_bps: discount_bps,
                auto_limit_bps: mandate.max_discount_without_approval_bps,
            },
        });
    }
    if days > (mandate.max_plan_days as i64) * 2 / 3 {
        return Some(PolicyDecision::RequireApproval {
            reason: ApprovalReason::PlanAboveAutoLimit {
                days,
                max_days: mandate.max_plan_days,
            },
        });
    }

    None
}

fn blocked(reason: BlockReason) -> PolicyDecision {
    PolicyDecision::Block { reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ClientId;
    use crate::invoice::{InvoiceLine, Quantity};
    use crate::money::Currency;
    use crate::negotiation::Installment;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn ts(day: u32, hour: u32) -> Timestamp {
        Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap()
    }

    fn day(d: u32) -> Day {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn invoice(total_minor: i64, status: InvoiceStatus) -> Invoice {
        let mut inv = Invoice::new(
            ClientId::new(),
            "1042",
            Currency::Usd,
            day(5),
            vec![InvoiceLine::new(
                "Design sprint",
                Quantity::whole(1).unwrap(),
                Money::new(total_minor, Currency::Usd),
            )],
            ts(1, 10),
        )
        .unwrap();
        match status {
            InvoiceStatus::Sent => inv.mark_sent(ts(1, 10)),
            InvoiceStatus::Overdue => {
                inv.mark_sent(ts(1, 10));
                inv.mark_overdue();
            }
            InvoiceStatus::Paid => {
                inv.mark_sent(ts(1, 10));
                inv.apply_payment(inv.total, ts(2, 10)).unwrap();
            }
            _ => {}
        }
        inv
    }

    /// A mandate whose contact window always allows, with fixed money limits.
    fn mandate() -> CollectionMandate {
        let mut m = CollectionMandate::default_for(Currency::Usd, ts(1, 10));
        m.contact_window = crate::mandate::ContactWindow {
            start_hour: 0,
            end_hour: 24,
            weekdays_only: false,
            utc_offset_minutes: 0,
        };
        m.min_days_between_contacts = 0;
        m
    }

    fn ctx<'a>(
        mandate: &'a CollectionMandate,
        invoice: &'a Invoice,
        actions: &'a [ActionSummary],
    ) -> PolicyContext<'a> {
        PolicyContext {
            now: ts(10, 12),
            mandate,
            invoice,
            recent_actions: actions,
            open_escalation: false,
        }
    }

    fn nudge() -> ProposedAction {
        ProposedAction::SendNudge {
            channel: Channel::Portal,
            tone: Tone::Warm,
        }
    }

    #[test]
    fn allows_a_plain_nudge_inside_limits() {
        let m = mandate();
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let decision = evaluate(&nudge(), &ctx(&m, &inv, &[]));
        assert!(decision.is_allow(), "{decision:?}");
    }

    #[test]
    fn observe_only_blocks_everything_outbound() {
        let mut m = mandate();
        m.autonomy = AutonomyLevel::Observe;
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        assert!(evaluate(&nudge(), &ctx(&m, &inv, &[])).is_blocked());
    }

    #[test]
    fn suggest_only_requires_approval() {
        let mut m = mandate();
        m.autonomy = AutonomyLevel::SuggestOnly;
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let decision = evaluate(&nudge(), &ctx(&m, &inv, &[]));
        assert!(matches!(
            decision,
            PolicyDecision::RequireApproval {
                reason: ApprovalReason::AutonomySuggestOnly
            }
        ));
    }

    #[test]
    fn weekly_contact_cap_blocks_the_third_nudge() {
        let m = mandate(); // cap = 2
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let actions = vec![
            ActionSummary::new(ActionKind::Nudge, ts(8, 12)),
            ActionSummary::new(ActionKind::Nudge, ts(9, 12)),
        ];
        let decision = evaluate(&nudge(), &ctx(&m, &inv, &actions));
        assert!(matches!(
            decision,
            PolicyDecision::Block {
                reason: BlockReason::WeeklyContactCap { used: 2, cap: 2 }
            }
        ));
    }

    #[test]
    fn minimum_gap_between_contacts_is_enforced() {
        let mut m = mandate();
        m.min_days_between_contacts = 3;
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let actions = vec![ActionSummary::new(ActionKind::Nudge, ts(9, 12))];
        let decision = evaluate(&nudge(), &ctx(&m, &inv, &actions));
        assert!(matches!(
            decision,
            PolicyDecision::Block {
                reason: BlockReason::TooSoon { .. }
            }
        ));
    }

    #[test]
    fn quiet_hours_block_human_channels_but_not_agents() {
        let mut m = mandate();
        m.contact_window = crate::mandate::ContactWindow {
            start_hour: 8,
            end_hour: 18,
            weekdays_only: false,
            utc_offset_minutes: 0,
        };
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let at_night = ts(10, 23);
        let ctx_night = PolicyContext {
            now: at_night,
            mandate: &m,
            invoice: &inv,
            recent_actions: &[],
            open_escalation: false,
        };
        assert!(evaluate(&nudge(), &ctx_night).is_blocked());

        let terms = Terms::immediate(Money::new(240_000, Currency::Usd));
        let a2a = ProposedAction::StartAgentNegotiation { terms };
        assert!(!evaluate(&a2a, &ctx_night).is_blocked());
    }

    #[test]
    fn settlement_floor_blocks_lowball_plans_but_allows_good_ones() {
        let m = mandate(); // floor 70 %
        let inv = invoice(240_000, InvoiceStatus::Overdue);

        let lowball = Terms {
            pay_now: Money::new(72_000, Currency::Usd), // 30 %
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(168_000, Currency::Usd),
                due: day(20),
            }],
            deadline: None,
        };
        let decision = evaluate(
            &ProposedAction::OfferTerms {
                terms: lowball,
                channel: Channel::Portal,
            },
            &ctx(&m, &inv, &[]),
        );
        assert!(matches!(
            decision,
            PolicyDecision::Block {
                reason: BlockReason::BelowSettlementFloor { .. }
            }
        ));

        let fair = Terms {
            pay_now: Money::new(120_000, Currency::Usd), // 50 % — still below 70 %
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(120_000, Currency::Usd),
                due: day(20),
            }],
            deadline: None,
        };
        assert!(evaluate(
            &ProposedAction::OfferTerms {
                terms: fair,
                channel: Channel::Portal,
            },
            &ctx(&m, &inv, &[]),
        )
        .is_blocked());

        let strong = Terms {
            pay_now: Money::new(170_000, Currency::Usd), // 70.83 %
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(70_000, Currency::Usd),
                due: day(20),
            }],
            deadline: None,
        };
        assert!(evaluate(
            &ProposedAction::OfferTerms {
                terms: strong,
                channel: Channel::Portal,
            },
            &ctx(&m, &inv, &[]),
        )
        .is_allow());
    }

    #[test]
    fn discounts_blocked_above_ceiling_and_ask_between_limits() {
        let mut m = mandate();
        m.max_discount_bps = 1_000; // 10 % ceiling
        m.max_discount_without_approval_bps = 200; // 2 % auto
        let inv = invoice(100_000, InvoiceStatus::Overdue);

        let too_much = ProposedAction::GrantDiscount {
            amount: Money::new(15_000, Currency::Usd), // 15 %
            reason: "client asked".into(),
        };
        assert!(matches!(
            evaluate(&too_much, &ctx(&m, &inv, &[])),
            PolicyDecision::Block {
                reason: BlockReason::DiscountAboveCeiling { .. }
            }
        ));

        let needs_approval = ProposedAction::GrantDiscount {
            amount: Money::new(5_000, Currency::Usd), // 5 %
            reason: "client asked".into(),
        };
        assert!(matches!(
            evaluate(&needs_approval, &ctx(&m, &inv, &[])),
            PolicyDecision::RequireApproval {
                reason: ApprovalReason::DiscountAboveAutoLimit { .. }
            }
        ));

        let auto_ok = ProposedAction::GrantDiscount {
            amount: Money::new(1_000, Currency::Usd), // 1 %
            reason: "rounding".into(),
        };
        assert!(evaluate(&auto_ok, &ctx(&m, &inv, &[])).is_allow());
    }

    #[test]
    fn plans_must_cover_the_debt_and_respect_shape_limits() {
        let m = mandate(); // max 3 installments, 45 days, min installment $50
        let inv = invoice(240_000, InvoiceStatus::Overdue);

        let not_covering = Terms {
            pay_now: Money::new(120_000, Currency::Usd),
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(60_000, Currency::Usd),
                due: day(20),
            }],
            deadline: None,
        };
        assert!(matches!(
            evaluate(
                &ProposedAction::OfferTerms {
                    terms: not_covering,
                    channel: Channel::Portal
                },
                &ctx(&m, &inv, &[])
            ),
            PolicyDecision::Block {
                reason: BlockReason::TermsDoNotCover { .. }
            }
        ));

        let too_many = Terms {
            pay_now: Money::new(180_000, Currency::Usd),
            discount: Money::zero(Currency::Usd),
            installments: (0..4)
                .map(|i| Installment {
                    amount: Money::new(15_000, Currency::Usd),
                    due: day(15 + i),
                })
                .collect(),
            deadline: None,
        };
        assert!(matches!(
            evaluate(
                &ProposedAction::IssuePlan { terms: too_many },
                &ctx(&m, &inv, &[])
            ),
            PolicyDecision::Block {
                reason: BlockReason::TooManyInstallments { .. }
            }
        ));

        let too_small = Terms {
            pay_now: Money::new(232_000, Currency::Usd),
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(8_000, Currency::Usd), // $80 < $50 min? no: 8000 = $80 > $50
                due: day(20),
            }],
            deadline: None,
        };
        assert!(evaluate(
            &ProposedAction::OfferTerms {
                terms: too_small,
                channel: Channel::Portal
            },
            &ctx(&m, &inv, &[])
        )
        .is_allow());

        let really_too_small = Terms {
            pay_now: Money::new(237_000, Currency::Usd),
            discount: Money::zero(Currency::Usd),
            installments: vec![Installment {
                amount: Money::new(3_000, Currency::Usd), // $30 < $50
                due: day(20),
            }],
            deadline: None,
        };
        assert!(matches!(
            evaluate(
                &ProposedAction::OfferTerms {
                    terms: really_too_small,
                    channel: Channel::Portal
                },
                &ctx(&m, &inv, &[])
            ),
            PolicyDecision::Block {
                reason: BlockReason::InstallmentTooSmall { .. }
            }
        ));
    }

    #[test]
    fn paid_invoices_block_new_actions() {
        let m = mandate();
        let inv = invoice(240_000, InvoiceStatus::Paid);
        assert!(matches!(
            evaluate(&nudge(), &ctx(&m, &inv, &[])),
            PolicyDecision::Block {
                reason: BlockReason::AlreadySettled
            }
        ));
    }

    #[test]
    fn escalation_requires_approval_and_cannot_duplicate() {
        let m = mandate();
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let escalation = ProposedAction::Escalate {
            reason: "four contacts, no reply".into(),
        };
        let decision = evaluate(&escalation, &ctx(&m, &inv, &[]));
        assert!(matches!(
            decision,
            PolicyDecision::RequireApproval {
                reason: ApprovalReason::Escalation
            }
        ));

        let mut with_open = ctx(&m, &inv, &[]);
        with_open.open_escalation = true;
        assert!(matches!(
            evaluate(&escalation, &with_open),
            PolicyDecision::Block {
                reason: BlockReason::EscalationAlreadyOpen
            }
        ));
    }

    #[test]
    fn agent_to_agent_respects_the_switch() {
        let mut m = mandate();
        m.allow_agent_to_agent = false;
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        let terms = Terms::immediate(Money::new(240_000, Currency::Usd));
        assert!(matches!(
            evaluate(
                &ProposedAction::StartAgentNegotiation { terms },
                &ctx(&m, &inv, &[])
            ),
            PolicyDecision::Block {
                reason: BlockReason::AgentToAgentDisabled
            }
        ));
    }

    #[test]
    fn closing_a_receivable_is_always_a_human_decision() {
        let m = mandate();
        let inv = invoice(240_000, InvoiceStatus::Overdue);
        assert!(matches!(
            evaluate(
                &ProposedAction::CloseUncollectible {
                    reason: "silent for 120 days".into()
                },
                &ctx(&m, &inv, &[])
            ),
            PolicyDecision::RequireApproval { .. }
        ));
    }
}
