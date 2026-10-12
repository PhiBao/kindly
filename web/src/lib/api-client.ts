/**
 * Typed client for the Kindly API.
 *
 * Everything the UI shows comes through here; there is no mock data in the
 * frontend. If the server is unreachable the UI says so instead of inventing
 * numbers.
 */

const BASE = process.env.NEXT_PUBLIC_KINDLY_API ?? "";

// ── types ─────────────────────────────────────────────────────────────────

export type PaypalMode = "simulation" | "live";

export interface Owner {
  name: string;
  business: string;
  invoicer_email: string;
  paypal_mode: PaypalMode;
  paypal_mode_raw: "sim" | "live";
  paypal_configured: boolean;
}

export interface Environment {
  model: string;
  model_enabled: boolean;
  paypal_base_url: string;
  public_url: string;
}

export interface Mandate {
  autonomy: "observe" | "suggest_only" | "act_within_limits";
  autonomy_label: string;
  autonomy_description: string;
  version: number;
  max_contacts_per_week: number;
  min_days_between_contacts: number;
  min_upfront_settlement_bps: number;
  max_discount_bps: number;
  max_discount_without_approval_bps: number;
  max_installments: number;
  max_plan_days: number;
  escalate_after_contacts: number;
  stop_after_days_silent: number;
  allow_agent_to_agent: boolean;
  contact_window: string;
  digest: string[];
}

export interface Stats {
  outstanding: string;
  outstanding_minor: number;
  collected: string;
  overdue_count: number;
  invoices_open: number;
  average_days_saved: number | null;
  agent_actions: number;
  paypal_events: number;
}

export interface ClientView {
  id: string;
  name: string;
  company: string | null;
  display_name: string;
  email: string;
  notes: string | null;
  avg_days_late: number;
  risk_label: string;
  invoices_paid: number;
}

export interface InvoiceSummary {
  id: string;
  number: string;
  status: string;
  status_label: string;
  currency: string;
  total: string;
  outstanding: string;
  outstanding_minor: number;
  total_minor: number;
  paid_minor: number;
  due_date: string;
  days_overdue: number;
  issued_at: string | null;
  paypal_invoice_id: string | null;
  client_id: string;
  client_name: string;
  client_contact: string;
  has_ap_agent: string;
  behavior: { avg_days_late: number; invoices_paid: number; risk: string; risk_label: string } | null;
  negotiation: {
    stage: string;
    stage_label: string;
    contacts: number;
    rounds: number;
    promised_on: string | null;
  };
  work_summary: string | null;
}

export interface AgentAction {
  id: string;
  invoice_id: string;
  kind: string;
  kind_label: string;
  status: string;
  status_label: string;
  proposal: Record<string, unknown>;
  decision: Record<string, unknown>;
  decision_explanation: string;
  rationale: string;
  created_at: string;
  executed_at: string | null;
  error: string | null;
}

export interface LedgerEvent {
  id?: string;
  at: string;
  actor?: string;
  actor_kind?: string;
  actor_label: string;
  kind: string;
  summary: string;
  detail?: Record<string, unknown> | null;
  invoice_id?: string | null;
  trust?: boolean;
}

export interface Attribution {
  invoice_id: string;
  invoice_number: string;
  client_name: string;
  kind: string;
  confidence: string;
  rationale: string;
  days_to_paid: number | null;
  days_saved: number | null;
  created_at: string;
}

export interface PaypalEvent {
  event_id: string;
  event_type: string;
  verified: boolean;
  status: string;
  received_at: string;
  error: string | null;
}

export interface KatpEntry {
  at: string;
  speaker: "kindly" | "client_agent" | "owner" | string;
  label: string;
  message: Record<string, unknown>;
  note: string;
  verified: boolean;
}

export interface A2aSession {
  id: string;
  invoice_id: string;
  invoice_number?: string;
  status: string;
  rounds: number;
  transcript: KatpEntry[];
  buyer: {
    company: string;
    agent_id: string;
    contact_name: string;
    public_key: string;
    policy: string[];
    max_unapproved_cash_minor: number;
    max_total_cash_minor: number;
    max_installments: number;
    payment_run_weekdays: number[];
    discount_ask_bps: number;
  };
  updated_at: string;
}

export interface Scenario {
  name: string;
  title: string;
  description: string;
  invoice: string;
}

export interface Bootstrap {
  owner: Owner;
  environment: Environment;
  mandate: Mandate;
  stats: Stats;
  clients: ClientView[];
  invoices: InvoiceSummary[];
  approvals: AgentAction[];
  actions: AgentAction[];
  ledger: LedgerEvent[];
  attributions: Attribution[];
  paypal_events: PaypalEvent[];
  sessions: A2aSession[];
  scenarios: Scenario[];
}

export interface Message {
  id?: string;
  author: string;
  author_label: string;
  subject?: string | null;
  body: string;
  intent?: Record<string, unknown> | null;
  at: string;
}

export interface InvoiceDetail {
  invoice: InvoiceSummary & {
    paypal_status: string | null;
    memo: string | null;
    lines: { description: string; quantity: string; unit_price: string; amount: string }[];
  };
  client: {
    id: string;
    name: string;
    display_name: string;
    email: string;
    notes: string | null;
    avg_days_late: number;
    risk_label: string;
  };
  negotiation: {
    stage: string;
    contact_count: number;
    rounds: number;
    promised_on: string | null;
    last_event_at: string;
  };
  actions: AgentAction[];
  ledger: LedgerEvent[];
  messages: Message[];
  payments: { at: string; amount: string; fully_settles: boolean; source: string }[];
  sessions: A2aSession[];
  portal_url: string;
}

export interface PortalView {
  invoice: {
    id: string;
    number: string;
    status: string;
    status_label: string;
    currency: string;
    total: string;
    outstanding: string;
    outstanding_minor: number;
    due_date: string;
    days_overdue: number;
    issued_at: string | null;
    work_summary: string | null;
    memo: string | null;
    lines: { description: string; quantity: string; amount: string }[];
    payments: { at: string; amount: string }[];
  };
  invoicer: { name: string; business: string };
  client: { contact: string; company: string };
  messages: Message[];
  negotiation: { stage: string; promised_on: string | null };
  paypal_enabled: boolean;
  simulated_payments: boolean;
}

// ── transport ─────────────────────────────────────────────────────────────

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${BASE}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
    cache: "no-store",
  });
  const text = await response.text();
  const payload = text ? JSON.parse(text) : {};
  if (!response.ok) {
    const message = payload?.error?.message ?? `request failed (${response.status})`;
    throw new Error(message);
  }
  return payload as T;
}

const post = <T>(path: string, body?: unknown) =>
  request<T>(path, { method: "POST", body: body === undefined ? undefined : JSON.stringify(body) });
const put = <T>(path: string, body?: unknown) =>
  request<T>(path, { method: "PUT", body: body === undefined ? undefined : JSON.stringify(body) });

export const api = {
  health: () =>
    request<{
      status: string;
      paypal_mode: string;
      paypal_configured: boolean;
      language_model: string;
      model_enabled: boolean;
      clients: number;
      invoices: number;
      ledger_events: number;
    }>("/api/health"),

  bootstrap: () => request<Bootstrap>("/api/bootstrap"),

  invoice: (id: string) => request<InvoiceDetail>(`/api/invoices/${id}`),

  ledger: (limit = 100) => request<LedgerEvent[]>(`/api/ledger?limit=${limit}`),

  tick: (id: string) => post<Record<string, unknown>>(`/api/invoices/${id}/tick`),

  tickAll: () => post<Record<string, unknown>>("/api/tick"),

  sendInvoice: (id: string) => post<Record<string, unknown>>(`/api/invoices/${id}/send`),

  remind: (id: string) => post<Record<string, unknown>>(`/api/invoices/${id}/remind`),

  decide: (actionId: string, approve: boolean) =>
    post<AgentAction>(`/api/actions/${actionId}/decision`, { approve }),

  mandate: (update: Partial<Mandate> & { contact_window_preset?: string }) =>
    put<Mandate>("/api/mandate", update),

  reset: () => post<Record<string, unknown>>("/api/demo/reset"),

  scenario: (name: string) => post<Record<string, unknown>>(`/api/demo/scenario/${name}`),

  a2aStep: (sessionId: string) => post<A2aSession>(`/api/a2a/${sessionId}/step`),

  a2aOwnerDecision: (sessionId: string, approve: boolean) =>
    post<A2aSession>(`/api/a2a/${sessionId}/owner-decision`, { approve }),

  a2aBuyerApproval: (sessionId: string) =>
    post<A2aSession>(`/api/a2a/${sessionId}/buyer-approval`),

  portal: (token: string) => request<PortalView>(`/api/portal/${token}`),

  portalMessage: (token: string, body: string) =>
    post<{ intent: string }>(`/api/portal/${token}/message`, { body }),

  portalOrder: (token: string, amountMinor?: number) =>
    post<{ order_id: string; amount_minor: number; currency: string; mode: string }>(
      `/api/portal/${token}/order`,
      amountMinor ? { amount_minor: amountMinor } : {},
    ),

  portalCapture: (token: string, orderId: string) =>
    post<{ captured: boolean; simulated: boolean }>(`/api/portal/${token}/capture`, {
      order_id: orderId,
    }),
};

// ── live events ───────────────────────────────────────────────────────────

export interface KindlyEvent {
  kind: string;
  at: string;
  payload: Record<string, unknown>;
}

/** Subscribe to the server's event stream. Returns an unsubscribe function. */
export function subscribeEvents(onEvent: (event: KindlyEvent) => void): () => void {
  if (typeof window === "undefined" || typeof EventSource === "undefined") return () => {};
  const source = new EventSource(`${BASE}/api/events`);
  source.addEventListener("kindly", (message) => {
    try {
      onEvent(JSON.parse((message as MessageEvent).data) as KindlyEvent);
    } catch {
      /* ignore malformed frames */
    }
  });
  return () => source.close();
}
