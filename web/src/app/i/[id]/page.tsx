"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { Badge, Empty, SectionLabel, Stamp, StatusLine } from "@/components/primitives";
import { LetterBody } from "@/components/letter-body";
import { api, type InvoiceDetail } from "@/lib/api-client";
import { longDate, relative, shortDate } from "@/lib/format";

export default function InvoiceDeskPage() {
  const params = useParams<{ id: string }>();
  const id = params.id;
  const [detail, setDetail] = useState<InvoiceDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setDetail(await api.invoice(id));
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [id]);

  useEffect(() => {
    void load();
    const interval = setInterval(() => void load(), 6000);
    return () => clearInterval(interval);
  }, [load]);

  const act = async (name: string, run: () => Promise<unknown>) => {
    setBusy(name);
    setNote(null);
    try {
      await run();
      await load();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  if (error) {
    return (
      <div className="mx-auto max-w-[900px] px-5 py-16">
        <div className="paper-edge p-5">
          <div className="eyebrow text-[var(--color-stamp)]">Invoice unavailable</div>
          <p className="mt-1 text-[0.875rem]">{error}</p>
          <Link className="btn mt-4 inline-flex" href="/">
            back to the desk
          </Link>
        </div>
      </div>
    );
  }

  if (!detail) {
    return <div className="mono-xs mx-auto max-w-[900px] px-5 py-16">Loading invoice…</div>;
  }

  const { invoice, client, negotiation, actions, ledger, messages, payments, sessions } = detail;
  const session = sessions[0];

  return (
    <div className="mx-auto w-full max-w-[1180px] px-5 pb-24 sm:px-8">
      <header className="rule-heavy-b flex flex-wrap items-end justify-between gap-4 pb-4 pt-7">
        <div>
          <Link href="/" className="eyebrow hover:text-[var(--color-ink)]">
            ← the desk
          </Link>
          <h1 className="display mt-2 text-[2rem] leading-none">
            Invoice <span className="num text-[1.85rem]">{invoice.number}</span>
          </h1>
          <p className="mt-1.5 text-[0.875rem] text-[var(--color-ink-2)]">
            {client.display_name} · {invoice.work_summary ?? invoice.memo ?? "—"}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2.5">
          <button className="btn btn--sm" onClick={() => void act("tick", () => api.tick(id))} disabled={busy !== null}>
            {busy === "tick" ? "thinking…" : "Run the agent on this"}
          </button>
          <button className="btn btn--sm" onClick={() => void act("send", () => api.sendInvoice(id))} disabled={busy !== null}>
            Send via PayPal
          </button>
          <button className="btn btn--sm" onClick={() => void act("remind", () => api.remind(id))} disabled={busy !== null}>
            PayPal reminder
          </button>
          <Link className="btn btn--sm" href="/analytics">
            Money view
          </Link>
        </div>
      </header>

      <div className="mt-8 grid gap-10 lg:grid-cols-[minmax(0,1.5fr)_minmax(0,1fr)]">
        <div className="space-y-9">
          <section className="paper-edge p-5">
            <div className="flex flex-wrap items-baseline justify-between gap-4">
              <div>
                <div className="eyebrow">Outstanding</div>
                <div className="num mt-1 text-[2.4rem] leading-none">{invoice.outstanding}</div>
                <div className="mono-xs mt-1.5">
                  of {invoice.total} · due {longDate(invoice.due_date)}
                  {invoice.days_overdue > 0 && ` · ${invoice.days_overdue} days late`}
                </div>
              </div>
              {invoice.status === "paid" ? (
                <Stamp label="Paid" tone="settled" live />
              ) : invoice.status === "overdue" ? (
                <Stamp label="Overdue" />
              ) : invoice.status === "partially_paid" ? (
                <Stamp label="Part paid" />
              ) : (
                <Stamp label="Sent" tone="quiet" />
              )}
            </div>

            <div className="mt-5 rule-heavy-t pt-3">
              <div className="eyebrow mb-2">Lines</div>
              <ul className="space-y-1.5">
                {invoice.lines.map((line) => (
                  <li key={line.description} className="flex items-baseline justify-between gap-4">
                    <span className="text-[0.8125rem]">{line.description}</span>
                    <span className="num text-[0.8125rem]">{line.amount}</span>
                  </li>
                ))}
              </ul>
            </div>

            {payments.length > 0 && (
              <div className="mt-4 rule-t pt-3">
                <div className="eyebrow mb-2">Payments received</div>
                <ul className="space-y-1">
                  {payments.map((payment, index) => (
                    <li key={index} className="flex items-baseline justify-between gap-3">
                      <span className="mono-xs">{longDate(payment.at)}</span>
                      <span className="num text-[0.8125rem] text-[var(--color-settled)]">
                        +{payment.amount} {payment.fully_settles ? "(settled)" : "(part)"}
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </section>

          <section>
            <SectionLabel aside={<span className="mono-xs">{messages.length} letter{messages.length === 1 ? "" : "s"}</span>}>
              The correspondence
            </SectionLabel>
            {messages.length === 0 ? (
              <Empty>No letters yet. Running the agent will write the first one.</Empty>
            ) : (
              <ol className="space-y-3.5">
                {messages.map((message, index) => (
                  <li
                    key={message.id ?? index}
                    className={`letter ${message.author === "client" ? "letter--client" : "letter--agent"}`}
                  >
                    <div className="mb-1.5 flex items-baseline justify-between gap-3">
                      <span className="eyebrow">
                        {message.author === "client" ? `${client.display_name} wrote` : "Kindly wrote — as you"}
                      </span>
                      <span className="mono-xs">{relative(message.at)}</span>
                    </div>
                    {message.subject && <div className="mb-1 text-[0.8125rem] font-medium">{message.subject}</div>}
                    <LetterBody>{message.body}</LetterBody>
                    {message.intent && (
                      <div className="mt-2.5 flex items-center gap-2">
                        <span className="mono-xs">read as</span>
                        <Badge tone="neutral">
                          {String((message.intent as { kind?: string }).kind ?? "").replace(/_/g, " ")}
                        </Badge>
                      </div>
                    )}
                  </li>
                ))}
              </ol>
            )}
          </section>

          {session && (
            <section>
              <SectionLabel aside={<Badge tone="neutral">{session.status.replace(/_/g, " ")}</Badge>}>
                Agent-to-agent
              </SectionLabel>
              <div className="paper-edge p-4">
                <p className="text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
                  {session.transcript[session.transcript.length - 1]?.note}
                </p>
                <Link className="btn btn--sm mt-3 inline-flex" href={`/n/${session.id}`}>
                  Open the negotiation room
                </Link>
              </div>
            </section>
          )}

          <section>
            <SectionLabel>The agent&rsquo;s record on this invoice</SectionLabel>
            {actions.length === 0 ? (
              <Empty>No actions yet.</Empty>
            ) : (
              <ul className="space-y-2.5">
                {[...actions].reverse().map((action) => (
                  <li key={action.id} className="rule-b pb-2.5">
                    <div className="flex items-baseline justify-between gap-3">
                      <span className="text-[0.8125rem] font-medium">{action.kind_label}</span>
                      <Badge
                        tone={
                          action.status === "executed"
                            ? "settled"
                            : action.status === "blocked"
                              ? "stamp"
                              : action.status === "awaiting_approval"
                                ? "amber"
                                : "neutral"
                        }
                      >
                        {action.status_label}
                      </Badge>
                    </div>
                    <p className="mono-xs mt-1">{action.decision_explanation}</p>
                    {action.error && <p className="mono-xs mt-1 text-[var(--color-stamp)]">{action.error}</p>}
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>

        <div className="space-y-9">
          <section>
            <SectionLabel>Facts</SectionLabel>
            <StatusLine label="Client" value={client.display_name} />
            <StatusLine label="Contact" value={client.name} />
            <StatusLine
              label="Their habit"
              value={`${client.risk_label}${client.avg_days_late > 0 ? `, ~${client.avg_days_late}d late` : ""}`}
            />
            <StatusLine label="PayPal invoice" value={invoice.paypal_invoice_id ?? "not created"} mono />
            <StatusLine label="PayPal status" value={invoice.paypal_status ?? "—"} mono />
            <StatusLine label="Issued" value={shortDate(invoice.issued_at)} />
            <StatusLine label="Stage" value={negotiation.stage.replace(/_/g, " ")} />
            <StatusLine label="Contacts" value={String(negotiation.contact_count)} />
            <StatusLine label="Negotiation rounds" value={String(negotiation.rounds)} />
            {negotiation.promised_on && <StatusLine label="Promised" value={longDate(negotiation.promised_on)} />}
            {client.notes && (
              <p className="mt-3 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">{client.notes}</p>
            )}
          </section>

          <section>
            <SectionLabel aside={<span className="mono-xs">what the client sees</span>}>Client link</SectionLabel>
            <p className="text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
              A signed, expiring link — no account needed. Open it yourself to read their side and pay as them.
            </p>
            <div className="mt-3 flex flex-wrap gap-2">
              <a className="btn btn--sm" href={detail.portal_url} target="_blank" rel="noreferrer">
                Open the portal
              </a>
              <button
                className="btn btn--sm"
                onClick={() => {
                  void navigator.clipboard?.writeText(detail.portal_url);
                  setNote("Client link copied.");
                }}
              >
                Copy link
              </button>
            </div>
            <p className="mono-xs mt-2 break-all">{detail.portal_url}</p>
            {note && <p className="mono-xs mt-2 text-[var(--color-ink-2)]">{note}</p>}
          </section>

          <section>
            <SectionLabel aside={<span className="mono-xs">append-only</span>}>Audit trail</SectionLabel>
            <ol className="space-y-2">
              {ledger.slice(0, 18).map((event, index) => (
                <li key={index} className="grid grid-cols-[4.6rem_1fr] gap-3 rule-b pb-1.5">
                  <span className="mono-xs pt-[0.15rem]">{relative(event.at)}</span>
                  <span className="text-[0.75rem] leading-snug">
                    {event.summary}
                    <span className="mono-xs mt-0.5 block">{event.actor_label}</span>
                  </span>
                </li>
              ))}
            </ol>
          </section>
        </div>
      </div>
    </div>
  );
}
