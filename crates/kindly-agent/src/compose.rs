//! Message composition: writing to a client in the owner's voice.
//!
//! Every message is either model-written (when a model is configured) or
//! rendered from a template. Both paths share the same fact sheet, so the
//! difference is voice quality, never content accuracy. Templates exist so the
//! product is fully usable with no API key at all.

use crate::llm::{complete_json, complete_text, LlmProvider, LlmRequest};
use crate::prompts;
use chrono::NaiveDate;
use kindly_core::mandate::Tone;
use kindly_core::money::Money;
use kindly_core::negotiation::EffectPurpose;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionSource {
    Model,
    Template,
}

impl CompositionSource {
    pub fn label(self) -> &'static str {
        match self {
            CompositionSource::Model => "written by the model",
            CompositionSource::Template => "written from your templates",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedMessage {
    pub subject: String,
    pub body: String,
    pub source: CompositionSource,
}

#[derive(Debug, Clone, Default)]
pub struct ComposeFacts {
    pub owner_name: String,
    pub client_contact: String,
    pub client_company: String,
    pub invoice_number: String,
    pub currency: String,
    pub amount_outstanding: Option<Money>,
    pub total: Option<Money>,
    pub due_date: Option<NaiveDate>,
    pub days_overdue: i64,
    pub work_summary: Option<String>,
    pub pay_link: String,
    pub plan_summary: Option<String>,
    pub promised_on: Option<NaiveDate>,
    /// Recent conversation, oldest first: (author, text).
    pub history: Vec<(String, String)>,
    pub language_hint: Option<String>,
}

impl ComposeFacts {
    fn render_map(&self) -> HashMap<&'static str, String> {
        let mut map = HashMap::new();
        map.insert("owner", display_first(&self.owner_name));
        map.insert("client_first", display_first(&self.client_contact));
        map.insert("company", self.client_company.clone());
        map.insert("invoice_number", self.invoice_number.clone());
        map.insert(
            "amount",
            self.amount_outstanding
                .map(|m| m.display_value())
                .unwrap_or_else(|| "0.00".into()),
        );
        map.insert(
            "total",
            self.total.map(|m| m.display_value()).unwrap_or_default(),
        );
        map.insert("currency", self.currency.clone());
        map.insert(
            "due_date",
            self.due_date
                .map(|d| d.format("%-d %B").to_string())
                .unwrap_or_default(),
        );
        map.insert("days_overdue", self.days_overdue.to_string());
        map.insert(
            "work",
            self.work_summary.clone().unwrap_or_else(|| "the work".into()),
        );
        map.insert("pay_link", self.pay_link.clone());
        map.insert(
            "plan_summary",
            self.plan_summary.clone().unwrap_or_default(),
        );
        map.insert(
            "promised_on",
            self.promised_on
                .map(|d| d.format("%A %-d %B").to_string())
                .unwrap_or_default(),
        );
        map
    }
}

fn display_first(name: &str) -> String {
    name.split_whitespace()
        .next()
        .unwrap_or(name)
        .to_string()
}

fn render(template: &str, facts: &ComposeFacts) -> String {
    let map = facts.render_map();
    let mut out = template.to_string();
    for (key, value) in map {
        out = out.replace(&format!("{{{{{key}}}}}"), &value);
    }
    out
}

/// The deterministic templates. Written to sound like a competent person, not
/// a billing system — this is the fallback voice, and it still has to be good.
pub fn template_for(purpose: EffectPurpose, tone: Tone, facts: &ComposeFacts) -> (String, String) {
    let (subject, body) = match purpose {
        EffectPurpose::FirstNudge => match tone {
            Tone::Warm => (
                "Invoice {{invoice_number}} — quick check-in",
                "Hi {{client_first}},\n\nJust a quiet note that invoice {{invoice_number}} for {{work}} ({{currency}}{{amount}}) was due on {{due_date}}. If it's already in your payment run, thank you — ignore me.\n\nIf not, you can settle it here: {{pay_link}}\n\n— {{owner}}",
            ),
            _ => (
                "Invoice {{invoice_number}} — quick check-in",
                "Hi {{client_first}},\n\nA note that invoice {{invoice_number}} ({{currency}}{{amount}}) was due on {{due_date}}. Here's the payment link if it's easiest: {{pay_link}}\n\nIf something's holding it up, tell me and we'll sort it out.\n\n— {{owner}}",
            ),
        },
        EffectPurpose::FollowUp => match tone {
            Tone::Firm | Tone::Formal => (
                "Invoice {{invoice_number}} — {{days_overdue}} days past due",
                "Hi {{client_first}},\n\nInvoice {{invoice_number}} ({{currency}}{{amount}}) is now {{days_overdue}} days past its due date of {{due_date}}. Please arrange payment this week, or tell me a date you can commit to.\n\nPayment link: {{pay_link}}\n\n— {{owner}}",
            ),
            _ => (
                "Following up: invoice {{invoice_number}}",
                "Hi {{client_first}},\n\nFollowing up on invoice {{invoice_number}} ({{currency}}{{amount}}), {{days_overdue}} days past due now. You can settle it here: {{pay_link}}\n\nIf it's queued for a payment run, just say the date and I'll stop chasing.\n\n— {{owner}}",
            ),
        },
        EffectPurpose::DeliveryOfPlan => (
            "Invoice {{invoice_number}} — plan as agreed",
            "Hi {{client_first}},\n\nAs agreed: {{plan_summary}}\n\nYou can make the first payment here: {{pay_link}}\n\nThanks for working this out with me.\n\n— {{owner}}",
        ),
        EffectPurpose::SchedulingConfirmation => (
            "Noted: {{promised_on}} for invoice {{invoice_number}}",
            "Hi {{client_first}},\n\nNoted — {{promised_on}} for invoice {{invoice_number}}. The link stays live if you want to settle early: {{pay_link}}\n\n— {{owner}}",
        ),
        EffectPurpose::Acknowledgement => (
            "Re: invoice {{invoice_number}}",
            "Thanks for getting back to me — noted.\n\n— {{owner}}",
        ),
        EffectPurpose::SecurityNotice => (
            "Re: invoice {{invoice_number}}",
            "Hi {{client_first}},\n\nI can only discuss paying invoice {{invoice_number}}. Changes to the amount or the terms are a decision for {{owner}}, not for me — but I'm happy to pass anything along.\n\nThe invoice is here: {{pay_link}}\n\n— {{owner}}",
        ),
        EffectPurpose::DisputeAcknowledgement => (
            "Invoice {{invoice_number}} — paused for review",
            "Thanks for flagging this. I've paused the reminders on invoice {{invoice_number}} and passed it to {{owner}} to look at.\n\n— {{owner}}",
        ),
    };
    (render(subject, facts), render(body, facts))
}

#[derive(Debug, Deserialize)]
struct ModelMessage {
    subject: String,
    body: String,
}

/// Compose the message for one purpose, preferring the model when available.
pub async fn compose_message(
    llm: &dyn LlmProvider,
    purpose: EffectPurpose,
    tone: Tone,
    facts: &ComposeFacts,
) -> ComposedMessage {
    if llm.is_enabled() {
        if let Some(message) = compose_with_model(llm, purpose, tone, facts).await {
            return ComposedMessage {
                subject: message.subject,
                body: message.body,
                source: CompositionSource::Model,
            };
        }
    }
    let (subject, body) = template_for(purpose, tone, facts);
    ComposedMessage {
        subject,
        body,
        source: CompositionSource::Template,
    }
}

async fn compose_with_model(
    llm: &dyn LlmProvider,
    purpose: EffectPurpose,
    tone: Tone,
    facts: &ComposeFacts,
) -> Option<ModelMessage> {
    let history = if facts.history.is_empty() {
        "(no earlier messages)".to_string()
    } else {
        facts.history
            .iter()
            .rev()
            .take(4)
            .rev()
            .map(|(author, text)| format!("{author}: {text}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let user = format!(
        r#"Write the next message to the client.

Purpose: {purpose:?}
Tone rung: {tone:?}

Facts (use exactly these; do not invent anything):
- Owner (you write as): {owner}
- Client contact: {client} {company}
- Invoice: {number}
- Amount outstanding: {currency}{amount} (invoice total {currency}{total})
- Due date: {due}
- Days overdue: {days}
- Work summary: {work}
- Payment link: {link}
- Agreed plan (if any): {plan}
- Promised payment date (if any): {promised}

Recent history (oldest to newest):
{history}

Return strict JSON: {{"subject": "...", "body": "..."}}
The body is a complete short email. Sign it as {owner}. Include the payment link naturally once.
If the client wrote in another language, reply in that language."#,
        purpose = purpose,
        tone = tone,
        owner = facts.owner_name,
        client = facts.client_contact,
        company = facts.client_company,
        number = facts.invoice_number,
        currency = facts.currency,
        amount = facts
            .amount_outstanding
            .map(|m| m.display_value())
            .unwrap_or_default(),
        total = facts.total.map(|m| m.display_value()).unwrap_or_default(),
        due = facts
            .due_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into()),
        days = facts.days_overdue,
        work = facts.work_summary.clone().unwrap_or_else(|| "the work".into()),
        link = facts.pay_link,
        plan = facts.plan_summary.clone().unwrap_or_else(|| "none".into()),
        promised = facts
            .promised_on
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "none".into()),
        history = history,
    );

    let system = format!("{}{}", prompts::HOUSE_STYLE, "\nReturn strict JSON only.");
    let message: ModelMessage = complete_json(llm, LlmRequest::json(system, user)).await?;
    if message.body.trim().is_empty() {
        return None;
    }
    Some(message)
}

/// One- or two-sentence summary of an invoice's state for the owner's ledger.
pub async fn brief_line(
    llm: &dyn LlmProvider,
    facts: &ComposeFacts,
    stage_label: &str,
) -> String {
    let fallback = if facts.days_overdue > 0 {
        if facts.promised_on.is_some() {
            format!(
                "{} · {} days past due · payment promised for {}",
                facts.invoice_number,
                facts.days_overdue,
                facts.promised_on.map(|d| d.format("%-d %B").to_string()).unwrap_or_default()
            )
        } else {
            format!(
                "{} · {} days past due · {}",
                facts.invoice_number, facts.days_overdue, stage_label
            )
        }
    } else {
        format!("{} · {}", facts.invoice_number, stage_label)
    };

    if !llm.is_enabled() {
        return fallback;
    }

    let user = format!(
        "Invoice {number}. Outstanding {currency}{amount}. Due {due}. {days} days past due. Current stage: {stage}. Client history: {history}.",
        number = facts.invoice_number,
        currency = facts.currency,
        amount = facts
            .amount_outstanding
            .map(|m| m.display_value())
            .unwrap_or_default(),
        due = facts
            .due_date
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into()),
        days = facts.days_overdue,
        stage = stage_label,
        history = if facts.history.is_empty() {
            "no prior messages".to_string()
        } else {
            format!("{} prior messages", facts.history.len())
        },
    );

    complete_text(llm, LlmRequest::text(prompts::DAILY_BRIEF_SYSTEM, user))
        .await
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::NullLlm;
    use kindly_core::money::Currency;

    fn facts() -> ComposeFacts {
        ComposeFacts {
            owner_name: "Maya Rodriguez".into(),
            client_contact: "Sam Chen".into(),
            client_company: "Acme Studio".into(),
            invoice_number: "1042".into(),
            currency: "USD".into(),
            amount_outstanding: Some(Money::new(240_000, Currency::Usd)),
            total: Some(Money::new(240_000, Currency::Usd)),
            due_date: NaiveDate::from_ymd_opt(2026, 10, 1),
            days_overdue: 11,
            work_summary: Some("the brand identity sprint".into()),
            pay_link: "https://kindly.demo/c/abc123".into(),
            plan_summary: Some("50% now, 50% on 24 October".into()),
            promised_on: Some(NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()),
            history: vec![],
            language_hint: None,
        }
    }

    #[test]
    fn templates_fill_every_placeholder() {
        let facts = facts();
        for purpose in [
            EffectPurpose::FirstNudge,
            EffectPurpose::FollowUp,
            EffectPurpose::DeliveryOfPlan,
            EffectPurpose::SchedulingConfirmation,
            EffectPurpose::Acknowledgement,
            EffectPurpose::SecurityNotice,
            EffectPurpose::DisputeAcknowledgement,
        ] {
            let (subject, body) = template_for(purpose, Tone::Warm, &facts);
            assert!(!subject.is_empty(), "subject missing for {purpose:?}");
            assert!(!body.contains("{{"), "unfilled placeholder in {purpose:?}: {body}");
            assert!(!subject.contains("{{"), "unfilled placeholder in subject: {subject}");
        }
    }

    #[test]
    fn plan_templates_include_the_plan_and_the_link() {
        let (_, body) = template_for(EffectPurpose::DeliveryOfPlan, Tone::Neutral, &facts());
        assert!(body.contains("50% now, 50% on 24 October"));
        assert!(body.contains("https://kindly.demo/c/abc123"));
    }

    #[tokio::test]
    async fn falls_back_to_templates_without_a_model() {
        let composed = compose_message(&NullLlm, EffectPurpose::FirstNudge, Tone::Warm, &facts()).await;
        assert_eq!(composed.source, CompositionSource::Template);
        assert!(composed.body.contains("Maya"));
        assert!(composed.body.contains("2,400.00"));
        assert!(composed.body.contains("1 October"));
    }
}
