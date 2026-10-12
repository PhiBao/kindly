-- Kindly schema.
--
-- Design notes:
-- * Money is always stored as integer minor units + an ISO currency code.
-- * Timestamps are RFC3339 strings (SQLite has no native date type).
-- * Domain objects that are always read as a whole (invoice lines, negotiation
--   state, action proposals) are stored as JSON alongside the columns that are
--   queried. That keeps the mapping code small and honest.
-- * `ledger_events` and `paypal_events` are append-only: nothing updates a
--   ledger row, ever.

CREATE TABLE IF NOT EXISTS clients (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    company           TEXT,
    email             TEXT NOT NULL,
    paypal_payer_id   TEXT,
    notes             TEXT,
    behavior_json     TEXT NOT NULL DEFAULT '{}',
    has_ap_agent      INTEGER NOT NULL DEFAULT 0,
    ap_agent_json     TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS invoices (
    id                TEXT PRIMARY KEY,
    client_id         TEXT NOT NULL REFERENCES clients(id),
    number            TEXT NOT NULL,
    status            TEXT NOT NULL,
    currency          TEXT NOT NULL,
    total_minor       INTEGER NOT NULL,
    paid_minor        INTEGER NOT NULL DEFAULT 0,
    due_date          TEXT NOT NULL,
    issued_at         TEXT,
    paid_at           TEXT,
    paypal_invoice_id TEXT,
    paypal_status     TEXT,
    memo              TEXT,
    work_summary      TEXT,
    parent_invoice_id TEXT,
    lines_json        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_invoices_client ON invoices(client_id);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);

CREATE TABLE IF NOT EXISTS mandates (
    id            TEXT PRIMARY KEY,
    version       INTEGER NOT NULL,
    payload_json  TEXT NOT NULL,
    is_active     INTEGER NOT NULL DEFAULT 1,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS negotiations (
    invoice_id    TEXT PRIMARY KEY REFERENCES invoices(id),
    payload_json  TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_actions (
    id               TEXT PRIMARY KEY,
    invoice_id       TEXT NOT NULL REFERENCES invoices(id),
    kind             TEXT NOT NULL,
    status           TEXT NOT NULL,
    proposal_json    TEXT NOT NULL,
    decision_json    TEXT NOT NULL,
    rationale        TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL UNIQUE,
    draft_message_id TEXT,
    created_at       TEXT NOT NULL,
    executed_at      TEXT,
    error            TEXT
);

CREATE INDEX IF NOT EXISTS idx_actions_invoice ON agent_actions(invoice_id, created_at);

CREATE TABLE IF NOT EXISTS threads (
    id           TEXT PRIMARY KEY,
    invoice_id   TEXT NOT NULL REFERENCES invoices(id),
    channel      TEXT NOT NULL,
    counterparty TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'open',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_threads_invoice ON threads(invoice_id);

CREATE TABLE IF NOT EXISTS messages (
    id           TEXT PRIMARY KEY,
    thread_id    TEXT NOT NULL REFERENCES threads(id),
    author       TEXT NOT NULL,
    author_label TEXT NOT NULL,
    subject      TEXT,
    body         TEXT NOT NULL,
    intent_json  TEXT,
    source       TEXT,
    created_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id, created_at);

CREATE TABLE IF NOT EXISTS payments (
    id            TEXT PRIMARY KEY,
    invoice_id    TEXT NOT NULL REFERENCES invoices(id),
    provider      TEXT NOT NULL,
    provider_ref  TEXT,
    amount_minor  INTEGER NOT NULL,
    currency      TEXT NOT NULL,
    kind          TEXT NOT NULL,
    received_at   TEXT NOT NULL,
    raw_json      TEXT
);

CREATE INDEX IF NOT EXISTS idx_payments_invoice ON payments(invoice_id);

CREATE TABLE IF NOT EXISTS paypal_events (
    id            TEXT PRIMARY KEY,
    event_id      TEXT NOT NULL UNIQUE,
    event_type    TEXT NOT NULL,
    verified      INTEGER NOT NULL,
    raw_json      TEXT NOT NULL,
    received_at   TEXT NOT NULL,
    processed_at  TEXT,
    status        TEXT NOT NULL,
    error         TEXT
);

CREATE TABLE IF NOT EXISTS ledger_events (
    id           TEXT PRIMARY KEY,
    at           TEXT NOT NULL,
    actor        TEXT NOT NULL,
    kind         TEXT NOT NULL,
    summary      TEXT NOT NULL,
    detail_json  TEXT,
    invoice_id   TEXT,
    client_id    TEXT
);

CREATE INDEX IF NOT EXISTS idx_ledger_at ON ledger_events(at DESC);

CREATE TABLE IF NOT EXISTS attributions (
    id               TEXT PRIMARY KEY,
    invoice_id       TEXT NOT NULL REFERENCES invoices(id),
    kind             TEXT NOT NULL,
    confidence       TEXT NOT NULL,
    action_ids_json  TEXT NOT NULL,
    rationale        TEXT NOT NULL,
    days_to_paid     INTEGER,
    baseline_days    INTEGER,
    days_saved       INTEGER,
    created_at       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plans (
    id           TEXT PRIMARY KEY,
    invoice_id   TEXT NOT NULL REFERENCES invoices(id),
    status       TEXT NOT NULL,
    terms_json   TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS a2a_sessions (
    id              TEXT PRIMARY KEY,
    invoice_id      TEXT NOT NULL REFERENCES invoices(id),
    status          TEXT NOT NULL,
    rounds          INTEGER NOT NULL DEFAULT 0,
    transcript_json TEXT NOT NULL,
    buyer_json      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
