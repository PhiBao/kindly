//! Persistence. One module, organised by entity, with typed reads and writes.
//!
//! Nothing above this layer knows about SQL. Nothing below it knows about the
//! agent. Domain objects are stored whole (JSON) beside the columns that are
//! actually queried.

use crate::errors::{ApiError, ApiResult};
use chrono::{DateTime, NaiveDate, Utc};
use kindly_core::action::{ActionKind, ActionStatus, AgentAction};
use kindly_core::client::{Client, PaymentBehavior};
use kindly_core::ids::{
    ActionId, Actor, ClientId, InvoiceId, InvoiceLineId, LedgerEventId, MessageId, PlanId, ThreadId,
};
use kindly_core::invoice::{Invoice, InvoiceLine, InvoiceStatus, Quantity};
use kindly_core::ledger::{LedgerEvent, LedgerKind};
use kindly_core::mandate::CollectionMandate;
use kindly_core::money::{Currency, Money};
use kindly_core::negotiation::{Negotiation, Terms};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

// ─────────────────────────────── helpers ───────────────────────────────

fn ts(value: &str) -> ApiResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| ApiError::Internal(format!("bad timestamp {value}: {e}")))
}

fn date(value: &str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|e| ApiError::Internal(format!("bad date {value}: {e}")))
}

fn json<T: serde::de::DeserializeOwned>(value: &str) -> ApiResult<T> {
    serde_json::from_str(value).map_err(|e| ApiError::Internal(format!("bad json column: {e}")))
}

fn to_json<T: Serialize>(value: &T) -> ApiResult<String> {
    serde_json::to_string(value).map_err(|e| ApiError::Internal(format!("json encode: {e}")))
}

// ─────────────────────────────── store ───────────────────────────────

#[derive(Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    // ── clients ──────────────────────────────────────────────────────

    pub async fn upsert_client(&self, client: &Client, has_ap_agent: bool) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO clients (id, name, company, email, paypal_payer_id, notes, behavior_json, has_ap_agent, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
               ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 company = excluded.company,
                 email = excluded.email,
                 paypal_payer_id = excluded.paypal_payer_id,
                 notes = excluded.notes,
                 behavior_json = excluded.behavior_json,
                 has_ap_agent = excluded.has_ap_agent,
                 updated_at = excluded.updated_at"#,
        )
        .bind(client.id.to_string())
        .bind(&client.name)
        .bind(&client.company)
        .bind(&client.email)
        .bind(&client.paypal_payer_id)
        .bind(&client.notes)
        .bind(to_json(&client.behavior)?)
        .bind(if has_ap_agent { 1 } else { 0 })
        .bind(client.created_at.to_rfc3339())
        .bind(client.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn client(&self, id: ClientId) -> ApiResult<Client> {
        let row = sqlx::query("SELECT * FROM clients WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        self.row_to_client(&row)
    }

    pub async fn clients(&self) -> ApiResult<Vec<Client>> {
        let rows = sqlx::query("SELECT * FROM clients ORDER BY COALESCE(company, name)")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| self.row_to_client(row)).collect()
    }

    pub async fn client_has_ap_agent(&self, id: ClientId) -> ApiResult<bool> {
        let row = sqlx::query("SELECT has_ap_agent FROM clients WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("has_ap_agent") == 1)
    }

    fn row_to_client(&self, row: &sqlx::sqlite::SqliteRow) -> ApiResult<Client> {
        let behavior: PaymentBehavior = json(row.get::<String, _>("behavior_json").as_str())?;
        Ok(Client {
            id: ClientId::from_uuid(
                row.get::<String, _>("id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("client id: {e}")))?,
            ),
            name: row.get("name"),
            company: row.get("company"),
            email: row.get("email"),
            paypal_payer_id: row.get("paypal_payer_id"),
            notes: row.get("notes"),
            behavior,
            created_at: ts(row.get::<String, _>("created_at").as_str())?,
            updated_at: ts(row.get::<String, _>("updated_at").as_str())?,
        })
    }

    // ── invoices ─────────────────────────────────────────────────────

    pub async fn upsert_invoice(
        &self,
        invoice: &Invoice,
        work_summary: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO invoices (id, client_id, number, status, currency, total_minor, paid_minor,
                                     due_date, issued_at, paid_at, paypal_invoice_id, paypal_status,
                                     memo, work_summary, parent_invoice_id, lines_json, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
               ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 paid_minor = excluded.paid_minor,
                 issued_at = excluded.issued_at,
                 paid_at = excluded.paid_at,
                 paypal_invoice_id = excluded.paypal_invoice_id,
                 paypal_status = excluded.paypal_status,
                 memo = excluded.memo,
                 work_summary = COALESCE(excluded.work_summary, invoices.work_summary),
                 parent_invoice_id = COALESCE(excluded.parent_invoice_id, invoices.parent_invoice_id),
                 lines_json = excluded.lines_json,
                 updated_at = excluded.updated_at"#,
        )
        .bind(invoice.id.to_string())
        .bind(invoice.client_id.to_string())
        .bind(&invoice.number)
        .bind(status_key(invoice.status))
        .bind(invoice.currency.code())
        .bind(invoice.total.minor)
        .bind(invoice.amount_paid.minor)
        .bind(invoice.due_date.format("%Y-%m-%d").to_string())
        .bind(invoice.issued_at.map(|d| d.to_rfc3339()))
        .bind(invoice.paid_at.map(|d| d.to_rfc3339()))
        .bind(&invoice.paypal_invoice_id)
        .bind(&invoice.paypal_status)
        .bind(&invoice.memo)
        .bind(work_summary)
        .bind(invoice.parent_invoice_id.map(|id| id.to_string()))
        .bind(to_json(&invoice.lines)?)
        .bind(invoice.created_at.to_rfc3339())
        .bind(invoice.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn invoice(&self, id: InvoiceId) -> ApiResult<Invoice> {
        let row = sqlx::query("SELECT * FROM invoices WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        self.row_to_invoice(&row)
    }

    pub async fn invoice_work_summary(&self, id: InvoiceId) -> ApiResult<Option<String>> {
        let row = sqlx::query("SELECT work_summary FROM invoices WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get("work_summary"))
    }

    pub async fn invoices(&self) -> ApiResult<Vec<Invoice>> {
        let rows = sqlx::query("SELECT * FROM invoices ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| self.row_to_invoice(row)).collect()
    }

    pub async fn invoices_for_client(&self, client_id: ClientId) -> ApiResult<Vec<Invoice>> {
        let rows = sqlx::query("SELECT * FROM invoices WHERE client_id = ?1 ORDER BY created_at DESC")
            .bind(client_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| self.row_to_invoice(row)).collect()
    }

    pub async fn invoice_by_paypal_id(&self, paypal_invoice_id: &str) -> ApiResult<Option<Invoice>> {
        let row = sqlx::query("SELECT * FROM invoices WHERE paypal_invoice_id = ?1")
            .bind(paypal_invoice_id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(|row| self.row_to_invoice(row)).transpose()
    }

    pub async fn open_invoices(&self) -> ApiResult<Vec<Invoice>> {
        let rows = sqlx::query(
            "SELECT * FROM invoices WHERE status IN ('sent','partially_paid','overdue') ORDER BY due_date ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|row| self.row_to_invoice(row)).collect()
    }

    fn row_to_invoice(&self, row: &sqlx::sqlite::SqliteRow) -> ApiResult<Invoice> {
        let currency = Currency::from_code(row.get::<String, _>("currency").as_str())?;
        let lines: Vec<InvoiceLine> = json(row.get::<String, _>("lines_json").as_str())?;
        let status = status_from_key(row.get::<String, _>("status").as_str());
        let issued_at: Option<String> = row.get("issued_at");
        let paid_at: Option<String> = row.get("paid_at");
        Ok(Invoice {
            id: InvoiceId::from_uuid(
                row.get::<String, _>("id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("invoice id: {e}")))?,
            ),
            client_id: ClientId::from_uuid(
                row.get::<String, _>("client_id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("client id: {e}")))?,
            ),
            number: row.get("number"),
            status,
            currency,
            lines,
            total: Money::new(row.get::<i64, _>("total_minor"), currency),
            amount_paid: Money::new(row.get::<i64, _>("paid_minor"), currency),
            due_date: date(row.get::<String, _>("due_date").as_str())?,
            issued_at: issued_at.as_deref().map(ts).transpose()?,
            paid_at: paid_at.as_deref().map(ts).transpose()?,
            paypal_invoice_id: row.get("paypal_invoice_id"),
            paypal_status: row.get("paypal_status"),
            memo: row.get("memo"),
            parent_invoice_id: row
                .get::<Option<String>, _>("parent_invoice_id")
                .and_then(|raw| raw.parse().ok())
                .map(InvoiceId::from_uuid),
            created_at: ts(row.get::<String, _>("created_at").as_str())?,
            updated_at: ts(row.get::<String, _>("updated_at").as_str())?,
        })
    }

    // ── mandate ──────────────────────────────────────────────────────

    pub async fn active_mandate(&self) -> ApiResult<CollectionMandate> {
        let row = sqlx::query("SELECT payload_json FROM mandates WHERE is_active = 1 ORDER BY version DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => json(row.get::<String, _>("payload_json").as_str()),
            None => Ok(CollectionMandate::default()),
        }
    }

    pub async fn save_mandate(&self, mandate: &CollectionMandate) -> ApiResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE mandates SET is_active = 0")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO mandates (id, version, payload_json, is_active, updated_at) VALUES (?1, ?2, ?3, 1, ?4)",
        )
        .bind(mandate.id.to_string())
        .bind(mandate.version as i64)
        .bind(to_json(mandate)?)
        .bind(mandate.updated_at.to_rfc3339())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // ── negotiation ──────────────────────────────────────────────────

    pub async fn save_negotiation(&self, negotiation: &Negotiation) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO negotiations (invoice_id, payload_json, updated_at) VALUES (?1, ?2, ?3)
               ON CONFLICT(invoice_id) DO UPDATE SET payload_json = excluded.payload_json, updated_at = excluded.updated_at"#,
        )
        .bind(negotiation.invoice_id.to_string())
        .bind(to_json(negotiation)?)
        .bind(negotiation.last_event_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn negotiation(&self, invoice_id: InvoiceId) -> ApiResult<Negotiation> {
        let row = sqlx::query("SELECT payload_json FROM negotiations WHERE invoice_id = ?1")
            .bind(invoice_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => json(row.get::<String, _>("payload_json").as_str()),
            None => Ok(Negotiation::new(invoice_id, Utc::now())),
        }
    }

    /// Invoices that are open, with their negotiation stage — the scheduler's
    /// worklist.
    pub async fn invoices_with_negotiation(&self) -> ApiResult<Vec<(Invoice, Negotiation)>> {
        let invoices = self.open_invoices().await?;
        let mut out = Vec::with_capacity(invoices.len());
        for invoice in invoices {
            let negotiation = self.negotiation(invoice.id).await?;
            out.push((invoice, negotiation));
        }
        Ok(out)
    }

    // ── agent actions ────────────────────────────────────────────────

    pub async fn save_action(&self, action: &AgentAction) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO agent_actions (id, invoice_id, kind, status, proposal_json, decision_json,
                                          rationale, idempotency_key, created_at, executed_at, error)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
               ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 decision_json = excluded.decision_json,
                 rationale = excluded.rationale,
                 executed_at = excluded.executed_at,
                 error = excluded.error"#,
        )
        .bind(action.id.to_string())
        .bind(action.invoice_id.to_string())
        .bind(action_kind_key(action.kind))
        .bind(action_status_key(action.status))
        .bind(to_json(&action.proposal)?)
        .bind(to_json(&action.decision)?)
        .bind(&action.rationale)
        .bind(&action.idempotency_key)
        .bind(action.created_at.to_rfc3339())
        .bind(action.executed_at.map(|d| d.to_rfc3339()))
        .bind(&action.error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn action(&self, id: ActionId) -> ApiResult<AgentAction> {
        let row = sqlx::query("SELECT * FROM agent_actions WHERE id = ?1")
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await?;
        self.row_to_action(&row)
    }

    pub async fn actions_for_invoice(&self, invoice_id: InvoiceId) -> ApiResult<Vec<AgentAction>> {
        let rows = sqlx::query("SELECT * FROM agent_actions WHERE invoice_id = ?1 ORDER BY created_at")
            .bind(invoice_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| self.row_to_action(row)).collect()
    }

    pub async fn actions(&self, limit: i64) -> ApiResult<Vec<AgentAction>> {
        let rows = sqlx::query("SELECT * FROM agent_actions ORDER BY created_at DESC LIMIT ?1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(|row| self.row_to_action(row)).collect()
    }

    /// Actions awaiting the owner's decision — the approval inbox.
    pub async fn pending_approvals(&self) -> ApiResult<Vec<AgentAction>> {
        let rows = sqlx::query(
            "SELECT * FROM agent_actions WHERE status = 'awaiting_approval' ORDER BY created_at ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|row| self.row_to_action(row)).collect()
    }

    /// Executed, client-facing actions, used by the policy engine's contact caps.
    pub async fn contact_history(
        &self,
        invoice_id: InvoiceId,
    ) -> ApiResult<Vec<kindly_core::policy::ActionSummary>> {
        let rows = sqlx::query(
            "SELECT kind, executed_at FROM agent_actions WHERE invoice_id = ?1 AND status = 'executed' AND executed_at IS NOT NULL",
        )
        .bind(invoice_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            let kind = action_kind_from_key(row.get::<String, _>("kind").as_str());
            let at = ts(row.get::<String, _>("executed_at").as_str())?;
            if kind.is_client_contact() {
                out.push(kindly_core::policy::ActionSummary::new(kind, at));
            }
        }
        Ok(out)
    }

    fn row_to_action(&self, row: &sqlx::sqlite::SqliteRow) -> ApiResult<AgentAction> {
        let executed_at: Option<String> = row.get("executed_at");
        Ok(AgentAction {
            id: ActionId::from_uuid(
                row.get::<String, _>("id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("action id: {e}")))?,
            ),
            invoice_id: InvoiceId::from_uuid(
                row.get::<String, _>("invoice_id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("invoice id: {e}")))?,
            ),
            kind: action_kind_from_key(row.get::<String, _>("kind").as_str()),
            status: action_status_from_key(row.get::<String, _>("status").as_str()),
            proposal: json(row.get::<String, _>("proposal_json").as_str())?,
            decision: json(row.get::<String, _>("decision_json").as_str())?,
            rationale: row.get("rationale"),
            idempotency_key: row.get("idempotency_key"),
            created_at: ts(row.get::<String, _>("created_at").as_str())?,
            executed_at: executed_at.as_deref().map(ts).transpose()?,
            error: row.get("error"),
        })
    }

    // ── threads & messages ───────────────────────────────────────────

    pub async fn set_action_draft_message(
        &self,
        action_id: ActionId,
        message_id: Option<MessageId>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE agent_actions SET draft_message_id = ?2 WHERE id = ?1")
            .bind(action_id.to_string())
            .bind(message_id.map(|id| id.to_string()))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn action_draft_message(&self, action_id: ActionId) -> ApiResult<Option<MessageId>> {
        let row = sqlx::query("SELECT draft_message_id FROM agent_actions WHERE id = ?1")
            .bind(action_id.to_string())
            .fetch_one(&self.pool)
            .await?;
        let raw: Option<String> = row.get("draft_message_id");
        Ok(raw.and_then(|value| value.parse().ok()).map(MessageId::from_uuid))
    }

    pub async fn set_message_source(&self, message_id: MessageId, source: &str) -> ApiResult<()> {
        sqlx::query("UPDATE messages SET source = ?2 WHERE id = ?1")
            .bind(message_id.to_string())
            .bind(source)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn ensure_thread(
        &self,
        invoice_id: InvoiceId,
        channel: &str,
        counterparty: &str,
    ) -> ApiResult<ThreadId> {
        let existing = sqlx::query("SELECT id FROM threads WHERE invoice_id = ?1 AND channel = ?2 LIMIT 1")
            .bind(invoice_id.to_string())
            .bind(channel)
            .fetch_optional(&self.pool)
            .await?;
        if let Some(row) = existing {
            return Ok(ThreadId::from_uuid(
                row.get::<String, _>("id")
                    .parse()
                    .map_err(|e| ApiError::Internal(format!("thread id: {e}")))?,
            ));
        }
        let id = ThreadId::new();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO threads (id, invoice_id, channel, counterparty, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'open', ?5, ?5)",
        )
        .bind(id.to_string())
        .bind(invoice_id.to_string())
        .bind(channel)
        .bind(counterparty)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn thread_for_invoice(&self, invoice_id: InvoiceId) -> ApiResult<Option<ThreadId>> {
        let row = sqlx::query("SELECT id FROM threads WHERE invoice_id = ?1 ORDER BY created_at LIMIT 1")
            .bind(invoice_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|row| {
            ThreadId::from_uuid(row.get::<String, _>("id").parse().unwrap_or_default())
        }))
    }

    pub async fn add_message(
        &self,
        thread_id: ThreadId,
        author: Actor,
        author_label: &str,
        subject: Option<&str>,
        body: &str,
        intent_json: Option<&str>,
        source: Option<&str>,
    ) -> ApiResult<MessageId> {
        self.add_message_at(
            thread_id,
            author,
            author_label,
            subject,
            body,
            intent_json,
            source,
            Utc::now(),
        )
        .await
    }

    /// Same as [`Store::add_message`], with an explicit timestamp so seeded
    /// history reads in the right order.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_message_at(
        &self,
        thread_id: ThreadId,
        author: Actor,
        author_label: &str,
        subject: Option<&str>,
        body: &str,
        intent_json: Option<&str>,
        source: Option<&str>,
        at: DateTime<Utc>,
    ) -> ApiResult<MessageId> {
        let id = MessageId::new();
        let now = at.to_rfc3339();
        sqlx::query(
            "INSERT INTO messages (id, thread_id, author, author_label, subject, body, intent_json, source, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .bind(id.to_string())
        .bind(thread_id.to_string())
        .bind(author.as_str())
        .bind(author_label)
        .bind(subject)
        .bind(body)
        .bind(intent_json)
        .bind(source)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        sqlx::query("UPDATE threads SET updated_at = ?2 WHERE id = ?1")
            .bind(thread_id.to_string())
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn message(&self, message_id: MessageId) -> ApiResult<Option<Message>> {
        let row = sqlx::query("SELECT * FROM messages WHERE id = ?1")
            .bind(message_id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref()
            .map(|row| {
                Ok(Message {
                    id: MessageId::from_uuid(
                        row.get::<String, _>("id")
                            .parse()
                            .map_err(|e| ApiError::Internal(format!("message id: {e}")))?,
                    ),
                    author: actor_from_key(row.get::<String, _>("author").as_str()),
                    author_label: row.get("author_label"),
                    subject: row.get("subject"),
                    body: row.get("body"),
                    intent_json: row.get("intent_json"),
                    source: row.get("source"),
                    created_at: ts(row.get::<String, _>("created_at").as_str())?,
                })
            })
            .transpose()
    }

    pub async fn messages(&self, thread_id: ThreadId) -> ApiResult<Vec<Message>> {
        let rows = sqlx::query("SELECT * FROM messages WHERE thread_id = ?1 ORDER BY created_at")
            .bind(thread_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter()
            .map(|row| {
                Ok(Message {
                    id: MessageId::from_uuid(
                        row.get::<String, _>("id")
                            .parse()
                            .map_err(|e| ApiError::Internal(format!("message id: {e}")))?,
                    ),
                    author: actor_from_key(row.get::<String, _>("author").as_str()),
                    author_label: row.get("author_label"),
                    subject: row.get("subject"),
                    body: row.get("body"),
                    intent_json: row.get("intent_json"),
                    source: row.get("source"),
                    created_at: ts(row.get::<String, _>("created_at").as_str())?,
                })
            })
            .collect()
    }

    // ── payments ─────────────────────────────────────────────────────

    pub async fn add_payment(
        &self,
        invoice_id: InvoiceId,
        provider_ref: Option<&str>,
        amount: Money,
        kind: &str,
        raw: Option<&serde_json::Value>,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO payments (id, invoice_id, provider, provider_ref, amount_minor, currency, kind, received_at, raw_json)
             VALUES (?1, ?2, 'paypal', ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(kindly_core::ids::PaymentId::new().to_string())
        .bind(invoice_id.to_string())
        .bind(provider_ref)
        .bind(amount.minor)
        .bind(amount.currency.code())
        .bind(kind)
        .bind(Utc::now().to_rfc3339())
        .bind(raw.map(|v| v.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn payments(
        &self,
        invoice_id: InvoiceId,
    ) -> ApiResult<Vec<kindly_core::attribution::PaymentRecord>> {
        let rows = sqlx::query("SELECT * FROM payments WHERE invoice_id = ?1 ORDER BY received_at")
            .bind(invoice_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for row in rows {
            let currency = Currency::from_code(row.get::<String, _>("currency").as_str())?;
            let kind: String = row.get("kind");
            out.push(kindly_core::attribution::PaymentRecord {
                at: ts(row.get::<String, _>("received_at").as_str())?,
                amount: Money::new(row.get::<i64, _>("amount_minor"), currency),
                fully_settles: kind == "settlement",
                source: match kind.as_str() {
                    "capture" => kindly_core::attribution::PaymentSource::PayPalOrder,
                    "recorded" => kindly_core::attribution::PaymentSource::RecordedExternal,
                    _ => kindly_core::attribution::PaymentSource::PayPalInvoice,
                },
            });
        }
        Ok(out)
    }

    /// Has a payment with this provider reference already been recorded?
    /// Used so a webhook and a direct API response cannot double-count.
    pub async fn has_payment_ref(&self, provider_ref: &str) -> ApiResult<bool> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM payments WHERE provider_ref = ?1")
            .bind(provider_ref)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n") > 0)
    }

    pub async fn payment_total_minor(&self, invoice_id: InvoiceId) -> ApiResult<i64> {
        let row = sqlx::query("SELECT COALESCE(SUM(amount_minor), 0) AS total FROM payments WHERE invoice_id = ?1")
            .bind(invoice_id.to_string())
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("total"))
    }

    // ── paypal events ────────────────────────────────────────────────

    /// Returns `false` when the event was already seen (idempotent intake).
    pub async fn record_paypal_event(
        &self,
        event_id: &str,
        event_type: &str,
        verified: bool,
        raw: &serde_json::Value,
    ) -> ApiResult<bool> {
        let result = sqlx::query(
            "INSERT OR IGNORE INTO paypal_events (id, event_id, event_type, verified, raw_json, received_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'received')",
        )
        .bind(kindly_core::ids::EventId::new().to_string())
        .bind(event_id)
        .bind(event_type)
        .bind(if verified { 1 } else { 0 })
        .bind(raw.to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn mark_paypal_event(
        &self,
        event_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> ApiResult<()> {
        sqlx::query("UPDATE paypal_events SET status = ?2, error = ?3, processed_at = ?4 WHERE event_id = ?1")
            .bind(event_id)
            .bind(status)
            .bind(error)
            .bind(Utc::now().to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn recent_paypal_events(&self, limit: i64) -> ApiResult<Vec<PayPalEventRow>> {
        let rows = sqlx::query("SELECT * FROM paypal_events ORDER BY received_at DESC LIMIT ?1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(PayPalEventRow {
                event_id: row.get("event_id"),
                event_type: row.get("event_type"),
                verified: row.get::<i64, _>("verified") == 1,
                status: row.get("status"),
                received_at: ts(row.get::<String, _>("received_at").as_str())?,
                error: row.get("error"),
            });
        }
        Ok(out)
    }

    // ── ledger ───────────────────────────────────────────────────────

    pub async fn append_ledger(&self, event: &LedgerEvent) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO ledger_events (id, at, actor, kind, summary, detail_json, invoice_id, client_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(event.id.to_string())
        .bind(event.at.to_rfc3339())
        .bind(event.actor.as_str())
        .bind(ledger_kind_key(event.kind))
        .bind(&event.summary)
        .bind(event.detail.as_ref().map(|d| d.to_string()))
        .bind(event.invoice_id.map(|id| id.to_string()))
        .bind(event.client_id.map(|id| id.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ledger(&self, limit: i64, invoice_id: Option<InvoiceId>) -> ApiResult<Vec<LedgerEvent>> {
        let rows = match invoice_id {
            Some(id) => {
                sqlx::query("SELECT * FROM ledger_events WHERE invoice_id = ?1 ORDER BY at DESC LIMIT ?2")
                    .bind(id.to_string())
                    .bind(limit)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query("SELECT * FROM ledger_events ORDER BY at DESC LIMIT ?1")
                    .bind(limit)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        let mut out = Vec::new();
        for row in rows {
            let at = ts(row.get::<String, _>("at").as_str())?;
            let actor = actor_from_key(row.get::<String, _>("actor").as_str());
            let kind = ledger_kind_from_key(row.get::<String, _>("kind").as_str());
            let summary: String = row.get("summary");
            let detail: Option<String> = row.get("detail_json");
            let invoice_id: Option<String> = row.get("invoice_id");
            let client_id: Option<String> = row.get("client_id");
            out.push(LedgerEvent {
                id: LedgerEventId::from_uuid(
                    row.get::<String, _>("id")
                        .parse()
                        .map_err(|e| ApiError::Internal(format!("ledger id: {e}")))?,
                ),
                at,
                actor,
                kind,
                summary,
                detail: detail.as_deref().and_then(|d| serde_json::from_str(d).ok()),
                invoice_id: invoice_id
                    .as_deref()
                    .and_then(|id| id.parse().ok())
                    .map(InvoiceId::from_uuid),
                client_id: client_id
                    .as_deref()
                    .and_then(|id| id.parse().ok())
                    .map(ClientId::from_uuid),
            });
        }
        Ok(out)
    }

    // ── attribution ──────────────────────────────────────────────────

    pub async fn save_attribution(
        &self,
        attribution: &kindly_core::attribution::Attribution,
    ) -> ApiResult<()> {
        sqlx::query(
            "INSERT INTO attributions (id, invoice_id, kind, confidence, action_ids_json, rationale,
                                       days_to_paid, baseline_days, days_saved, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .bind(attribution.id.to_string())
        .bind(attribution.invoice_id.to_string())
        .bind(attribution_kind_key(attribution.kind))
        .bind(confidence_key(attribution.confidence))
        .bind(to_json(&attribution.action_ids)?)
        .bind(&attribution.rationale)
        .bind(attribution.days_to_paid)
        .bind(attribution.baseline_days)
        .bind(attribution.days_saved)
        .bind(attribution.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn attributions(&self, limit: i64) -> ApiResult<Vec<AttributionRow>> {
        let rows = sqlx::query(
            "SELECT a.*, COALESCE(c.company, c.name) AS client_name, i.number AS invoice_number
             FROM attributions a
             JOIN invoices i ON i.id = a.invoice_id
             JOIN clients c ON c.id = i.client_id
             ORDER BY a.created_at DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(AttributionRow {
                invoice_id: row.get::<String, _>("invoice_id"),
                invoice_number: row.get("invoice_number"),
                client_name: row.get("client_name"),
                kind: attribution_kind_from_key(row.get::<String, _>("kind").as_str()),
                confidence: row.get("confidence"),
                rationale: row.get("rationale"),
                days_to_paid: row.get("days_to_paid"),
                days_saved: row.get("days_saved"),
                created_at: ts(row.get::<String, _>("created_at").as_str())?,
            });
        }
        Ok(out)
    }

    // ── plans ────────────────────────────────────────────────────────

    pub async fn save_plan(
        &self,
        id: PlanId,
        invoice_id: InvoiceId,
        status: &str,
        terms: &Terms,
    ) -> ApiResult<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            r#"INSERT INTO plans (id, invoice_id, status, terms_json, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?5)
               ON CONFLICT(id) DO UPDATE SET status = excluded.status, terms_json = excluded.terms_json, updated_at = excluded.updated_at"#,
        )
        .bind(id.to_string())
        .bind(invoice_id.to_string())
        .bind(status)
        .bind(to_json(terms)?)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn plans(&self) -> ApiResult<Vec<PlanRow>> {
        let rows = sqlx::query("SELECT * FROM plans ORDER BY created_at DESC").fetch_all(&self.pool).await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(PlanRow {
                id: row.get::<String, _>("id"),
                invoice_id: row.get::<String, _>("invoice_id"),
                status: row.get("status"),
                terms: json(row.get::<String, _>("terms_json").as_str())?,
                created_at: ts(row.get::<String, _>("created_at").as_str())?,
            });
        }
        Ok(out)
    }

    // ── a2a sessions ─────────────────────────────────────────────────

    pub async fn save_a2a_session(&self, session: &A2aSessionRow) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO a2a_sessions (id, invoice_id, status, rounds, transcript_json, buyer_json, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
               ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 rounds = excluded.rounds,
                 transcript_json = excluded.transcript_json,
                 buyer_json = excluded.buyer_json,
                 updated_at = excluded.updated_at"#,
        )
        .bind(&session.id)
        .bind(&session.invoice_id)
        .bind(&session.status)
        .bind(session.rounds)
        .bind(to_json(&session.transcript)?)
        .bind(to_json(&session.buyer)?)
        .bind(session.created_at.to_rfc3339())
        .bind(session.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn a2a_session(&self, id: &str) -> ApiResult<A2aSessionRow> {
        let row = sqlx::query("SELECT * FROM a2a_sessions WHERE id = ?1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(A2aSessionRow {
            id: row.get("id"),
            invoice_id: row.get("invoice_id"),
            status: row.get("status"),
            rounds: row.get("rounds"),
            transcript: json(row.get::<String, _>("transcript_json").as_str())?,
            buyer: json(row.get::<String, _>("buyer_json").as_str())?,
            created_at: ts(row.get::<String, _>("created_at").as_str())?,
            updated_at: ts(row.get::<String, _>("updated_at").as_str())?,
        })
    }

    pub async fn a2a_sessions_for_invoice(&self, invoice_id: InvoiceId) -> ApiResult<Vec<A2aSessionRow>> {
        let rows = sqlx::query("SELECT * FROM a2a_sessions WHERE invoice_id = ?1 ORDER BY created_at DESC")
            .bind(invoice_id.to_string())
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(A2aSessionRow {
                id: row.get("id"),
                invoice_id: row.get("invoice_id"),
                status: row.get("status"),
                rounds: row.get("rounds"),
                transcript: json(row.get::<String, _>("transcript_json").as_str())?,
                buyer: json(row.get::<String, _>("buyer_json").as_str())?,
                created_at: ts(row.get::<String, _>("created_at").as_str())?,
                updated_at: ts(row.get::<String, _>("updated_at").as_str())?,
            });
        }
        Ok(out)
    }

    /// Demo/dev helper: drop the agent-to-agent history for one invoice so a
    /// scenario can be replayed from the start.
    pub async fn delete_a2a_sessions(&self, invoice_id: InvoiceId) -> ApiResult<()> {
        sqlx::query("DELETE FROM a2a_sessions WHERE invoice_id = ?1")
            .bind(invoice_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_rows(&self, table: &str) -> ApiResult<i64> {
        // table names are internal constants, never user input
        let sql = format!("SELECT COUNT(*) AS n FROM {table}");
        let row = sqlx::query(&sql).fetch_one(&self.pool).await?;
        Ok(row.get::<i64, _>("n"))
    }

    pub async fn clear_all(&self) -> ApiResult<()> {
        let mut tx = self.pool.begin().await?;
        for table in [
            "a2a_sessions",
            "plans",
            "attributions",
            "ledger_events",
            "paypal_events",
            "payments",
            "messages",
            "threads",
            "agent_actions",
            "negotiations",
            "invoices",
            "clients",
            "mandates",
            "settings",
        ] {
            sqlx::query(&format!("DELETE FROM {table}"))
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

// ─────────────────────────────── rows ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub author: Actor,
    pub author_label: String,
    pub subject: Option<String>,
    pub body: String,
    pub intent_json: Option<String>,
    pub source: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PayPalEventRow {
    pub event_id: String,
    pub event_type: String,
    pub verified: bool,
    pub status: String,
    pub received_at: DateTime<Utc>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttributionRow {
    pub invoice_id: String,
    pub invoice_number: String,
    pub client_name: String,
    pub kind: String,
    pub confidence: String,
    pub rationale: String,
    pub days_to_paid: Option<i64>,
    pub days_saved: Option<i64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanRow {
    pub id: String,
    pub invoice_id: String,
    pub status: String,
    pub terms: Terms,
    pub created_at: DateTime<Utc>,
}

/// The buyer agent's persistent bits (its signing key lives in memory only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aBuyerRow {
    pub company: String,
    pub agent_id: String,
    pub contact_name: String,
    pub public_key: String,
    pub policy: Vec<String>,
    pub max_unapproved_cash_minor: i64,
    pub max_total_cash_minor: i64,
    pub max_installments: u8,
    pub payment_run_weekdays: Vec<u8>,
    pub discount_ask_bps: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aTranscriptEntry {
    pub at: DateTime<Utc>,
    pub speaker: String,
    pub label: String,
    pub message: serde_json::Value,
    pub note: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aSessionRow {
    pub id: String,
    pub invoice_id: String,
    pub status: String,
    pub rounds: i64,
    pub transcript: Vec<A2aTranscriptEntry>,
    pub buyer: A2aBuyerRow,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─────────────────────────────── key mapping ───────────────────────────────

pub fn status_key(status: InvoiceStatus) -> &'static str {
    match status {
        InvoiceStatus::Draft => "draft",
        InvoiceStatus::Sent => "sent",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::Cancelled => "cancelled",
        InvoiceStatus::Overdue => "overdue",
    }
}

pub fn status_from_key(key: &str) -> InvoiceStatus {
    match key {
        "draft" => InvoiceStatus::Draft,
        "sent" => InvoiceStatus::Sent,
        "partially_paid" => InvoiceStatus::PartiallyPaid,
        "paid" => InvoiceStatus::Paid,
        "cancelled" => InvoiceStatus::Cancelled,
        _ => InvoiceStatus::Overdue,
    }
}

pub fn action_kind_key(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Nudge => "nudge",
        ActionKind::OfferTerms => "offer_terms",
        ActionKind::IssuePlan => "issue_plan",
        ActionKind::RecordPartialPayment => "record_partial_payment",
        ActionKind::GrantDiscount => "grant_discount",
        ActionKind::Escalate => "escalate",
        ActionKind::Pause => "pause",
        ActionKind::StartAgentNegotiation => "start_agent_negotiation",
        ActionKind::CloseUncollectible => "close_uncollectible",
    }
}

pub fn action_kind_from_key(key: &str) -> ActionKind {
    match key {
        "nudge" => ActionKind::Nudge,
        "offer_terms" => ActionKind::OfferTerms,
        "issue_plan" => ActionKind::IssuePlan,
        "record_partial_payment" => ActionKind::RecordPartialPayment,
        "grant_discount" => ActionKind::GrantDiscount,
        "escalate" => ActionKind::Escalate,
        "pause" => ActionKind::Pause,
        "start_agent_negotiation" => ActionKind::StartAgentNegotiation,
        _ => ActionKind::CloseUncollectible,
    }
}

pub fn action_status_key(status: ActionStatus) -> &'static str {
    match status {
        ActionStatus::Proposed => "proposed",
        ActionStatus::AwaitingApproval => "awaiting_approval",
        ActionStatus::Approved => "approved",
        ActionStatus::Executing => "executing",
        ActionStatus::Executed => "executed",
        ActionStatus::Failed => "failed",
        ActionStatus::Blocked => "blocked",
        ActionStatus::Rejected => "rejected",
    }
}

pub fn action_status_from_key(key: &str) -> ActionStatus {
    match key {
        "proposed" => ActionStatus::Proposed,
        "awaiting_approval" => ActionStatus::AwaitingApproval,
        "approved" => ActionStatus::Approved,
        "executing" => ActionStatus::Executing,
        "executed" => ActionStatus::Executed,
        "failed" => ActionStatus::Failed,
        "blocked" => ActionStatus::Blocked,
        _ => ActionStatus::Rejected,
    }
}

pub fn ledger_kind_key(kind: LedgerKind) -> &'static str {
    match kind {
        LedgerKind::InvoiceCreated => "invoice_created",
        LedgerKind::InvoiceSent => "invoice_sent",
        LedgerKind::InvoicePaid => "invoice_paid",
        LedgerKind::InvoicePartiallyPaid => "invoice_partially_paid",
        LedgerKind::PaymentRecorded => "payment_recorded",
        LedgerKind::AgentProposed => "agent_proposed",
        LedgerKind::PolicyAllowed => "policy_allowed",
        LedgerKind::PolicyBlocked => "policy_blocked",
        LedgerKind::PolicyApprovalRequested => "policy_approval_requested",
        LedgerKind::PolicyApprovalGranted => "policy_approval_granted",
        LedgerKind::PolicyApprovalRejected => "policy_approval_rejected",
        LedgerKind::ActionExecuted => "action_executed",
        LedgerKind::ActionFailed => "action_failed",
        LedgerKind::MessageSent => "message_sent",
        LedgerKind::MessageReceived => "message_received",
        LedgerKind::NegotiationEvent => "negotiation_event",
        LedgerKind::PlanIssued => "plan_issued",
        LedgerKind::AgentToAgentStep => "agent_to_agent_step",
        LedgerKind::SecurityEvent => "security_event",
        LedgerKind::MandateUpdated => "mandate_updated",
        LedgerKind::PayPalCall => "paypal_call",
        LedgerKind::PayPalWebhook => "paypal_webhook",
        LedgerKind::ForecastUpdated => "forecast_updated",
        LedgerKind::System => "system",
    }
}

pub fn ledger_kind_from_key(key: &str) -> LedgerKind {
    match key {
        "invoice_created" => LedgerKind::InvoiceCreated,
        "invoice_sent" => LedgerKind::InvoiceSent,
        "invoice_paid" => LedgerKind::InvoicePaid,
        "invoice_partially_paid" => LedgerKind::InvoicePartiallyPaid,
        "payment_recorded" => LedgerKind::PaymentRecorded,
        "agent_proposed" => LedgerKind::AgentProposed,
        "policy_allowed" => LedgerKind::PolicyAllowed,
        "policy_blocked" => LedgerKind::PolicyBlocked,
        "policy_approval_requested" => LedgerKind::PolicyApprovalRequested,
        "policy_approval_granted" => LedgerKind::PolicyApprovalGranted,
        "policy_approval_rejected" => LedgerKind::PolicyApprovalRejected,
        "action_executed" => LedgerKind::ActionExecuted,
        "action_failed" => LedgerKind::ActionFailed,
        "message_sent" => LedgerKind::MessageSent,
        "message_received" => LedgerKind::MessageReceived,
        "negotiation_event" => LedgerKind::NegotiationEvent,
        "plan_issued" => LedgerKind::PlanIssued,
        "agent_to_agent_step" => LedgerKind::AgentToAgentStep,
        "security_event" => LedgerKind::SecurityEvent,
        "mandate_updated" => LedgerKind::MandateUpdated,
        "paypal_call" => LedgerKind::PayPalCall,
        "paypal_webhook" => LedgerKind::PayPalWebhook,
        "forecast_updated" => LedgerKind::ForecastUpdated,
        _ => LedgerKind::System,
    }
}

pub fn actor_from_key(key: &str) -> Actor {
    match key {
        "owner" => Actor::Owner,
        "kindly_agent" => Actor::KindlyAgent,
        "client" => Actor::Client,
        "client_agent" => Actor::ClientAgent,
        "paypal" => Actor::PayPal,
        _ => Actor::System,
    }
}

pub fn attribution_kind_key(kind: kindly_core::attribution::CauseKind) -> &'static str {
    use kindly_core::attribution::CauseKind;
    match kind {
        CauseKind::PlanSettled => "plan_settled",
        CauseKind::ReminderLed => "reminder_led",
        CauseKind::PromiseKept => "promise_kept",
        CauseKind::Organic => "organic",
        CauseKind::Unknown => "unknown",
    }
}

pub fn attribution_kind_from_key(key: &str) -> String {
    key.to_string()
}

pub fn confidence_key(confidence: kindly_core::attribution::Confidence) -> &'static str {
    use kindly_core::attribution::Confidence;
    match confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

/// Rebuild a domain line from stored JSON (used by the seed and by tests).
pub fn line_from_parts(
    description: &str,
    quantity_thousandths: i64,
    unit_minor: i64,
    currency: Currency,
) -> ApiResult<InvoiceLine> {
    Ok(InvoiceLine {
        id: InvoiceLineId::new(),
        description: description.to_string(),
        quantity: Quantity::from_thousandths(quantity_thousandths)
            .map_err(|e| ApiError::Domain(e))?,
        unit_price: Money::new(unit_minor, currency),
    })
}

impl Store {
    pub async fn set_setting(&self, key: &str, value: &serde_json::Value) -> ApiResult<()> {
        sqlx::query(
            r#"INSERT INTO settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)
               ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at"#,
        )
        .bind(key)
        .bind(value.to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> ApiResult<Option<serde_json::Value>> {
        let row = sqlx::query("SELECT value_json FROM settings WHERE key = ?1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|row| {
            let raw: String = row.get("value_json");
            serde_json::from_str(&raw).ok()
        }))
    }
}
