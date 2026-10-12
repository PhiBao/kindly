"use client";

import { useParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { LetterBody } from "@/components/letter-body";
import { Stamp } from "@/components/primitives";
import { api, type PortalView } from "@/lib/api-client";
import { longDate, money, relative } from "@/lib/format";

type Phase = "idle" | "paying" | "paid" | "error";

export default function ClientPortalPage() {
  const params = useParams<{ token: string }>();
  const token = params.token;

  const [view, setView] = useState<PortalView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [note, setNote] = useState<string | null>(null);
  const [amount, setAmount] = useState<string>("");
  const [message, setMessage] = useState("");
  const [sending, setSending] = useState(false);
  const [justPaid, setJustPaid] = useState(false);
  const paidRef = useRef(false);

  const load = useCallback(async () => {
    try {
      const next = await api.portal(token);
      setView(next);
      setError(null);
      if (paidRef.current && next.invoice.outstanding_minor === 0) {
        setPhase("paid");
      }
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [token]);

  useEffect(() => {
    void load();
    const interval = setInterval(() => void load(), 5000);
    return () => clearInterval(interval);
  }, [load]);

  const pay = async (amountMinor?: number) => {
    if (!view) return;
    setPhase("paying");
    setNote(null);
    try {
      const order = await api.portalOrder(token, amountMinor);
      const capture = await api.portalCapture(token, order.order_id);
      paidRef.current = true;
      setJustPaid(true);
      setNote(
        capture.simulated
          ? "Payment recorded through Kindly's PayPal simulator — no real money moved."
          : "Payment captured by PayPal.",
      );
      await load();
      setPhase(view.invoice.outstanding_minor - (amountMinor ?? view.invoice.outstanding_minor) <= 0 ? "paid" : "idle");
    } catch (cause) {
      setPhase("error");
      setNote(cause instanceof Error ? cause.message : String(cause));
    }
  };

  const sendMessage = async () => {
    if (!view || message.trim().length === 0) return;
    setSending(true);
    setNote(null);
    try {
      const result = await api.portalMessage(token, message.trim());
      setMessage("");
      setNote(`Kindly read that as: ${result.intent}.`);
      await load();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSending(false);
    }
  };

  if (error) {
    return (
      <div className="mx-auto max-w-[640px] px-5 py-20">
        <div className="paper-edge p-6">
          <div className="eyebrow text-[var(--color-stamp)]">This link cannot be used</div>
          <p className="mt-2 text-[0.875rem] leading-relaxed">{error}</p>
          <p className="mono-xs mt-3">
            Payment links expire and are signed to one invoice. Ask {""}
            the sender for a fresh one.
          </p>
        </div>
      </div>
    );
  }

  if (!view) {
    return <div className="mono-xs px-5 py-20 text-center">Opening your invoice…</div>;
  }

  const { invoice, invoicer, client, messages, simulated_payments } = view;
  const settled = invoice.outstanding_minor === 0;
  const partial = !settled && invoice.payments.length > 0;
  const dueAmount = Number(amount.replace(/[^0-9.]/g, ""));
  const parsedMinor = Number.isFinite(dueAmount) && dueAmount > 0 ? Math.round(dueAmount * 100) : null;
  const validPartial = parsedMinor !== null && parsedMinor <= invoice.outstanding_minor;

  return (
    <div className="mx-auto w-full max-w-[720px] px-5 pb-20 pt-12 sm:pt-16">
      <header className="text-center">
        <div className="eyebrow">{invoicer.business}</div>
        <h1 className="display mt-2 text-[2.4rem] leading-none">
          Invoice <span className="num text-[2.1rem]">{invoice.number}</span>
        </h1>
        <p className="mt-2 text-[0.875rem] text-[var(--color-ink-2)]">
          For {client.company} · due {longDate(invoice.due_date)}
          {invoice.days_overdue > 0 && !settled && ` · ${invoice.days_overdue} days ago`}
        </p>
      </header>

      <section className="paper-edge mt-8 p-6 sm:p-7">
        <div className="flex flex-wrap items-start justify-between gap-5">
          <div>
            <div className="eyebrow">{settled ? "Settled" : partial ? "Still outstanding" : "Amount due"}</div>
            <div className="num mt-1.5 text-[2.9rem] leading-none">
              {settled ? invoice.total : invoice.outstanding}
            </div>
            {partial && (
              <div className="mono-xs mt-1.5">
                of {invoice.total} · {invoice.payments.length} payment(s) already received
              </div>
            )}
          </div>
          {settled ? (
            <Stamp label="Paid" tone="settled" live={justPaid} />
          ) : partial ? (
            <Stamp label="Part paid" />
          ) : null}
        </div>

        {!settled && (
          <div className="mt-6 rule-heavy-t pt-5">
            <div className="flex flex-wrap items-center gap-3">
              <button
                className="btn btn--solid !px-5 !py-2.5 !text-[0.875rem]"
                onClick={() => void pay()}
                disabled={phase === "paying"}
              >
                {phase === "paying"
                  ? "Opening PayPal…"
                  : `Pay ${invoice.outstanding} with PayPal`}
              </button>
              {simulated_payments && (
                <span className="mono-xs">
                  This deployment runs PayPal in simulation — the flow is real, the money is not.
                </span>
              )}
            </div>

            <div className="mt-4 flex flex-wrap items-center gap-2">
              <span className="mono-xs">or pay part of it:</span>
              <input
                className="field num !w-[9rem]"
                placeholder={money(invoice.outstanding_minor / 2, invoice.currency).replace("$", "")}
                value={amount}
                onChange={(event) => setAmount(event.target.value)}
              />
              <button
                className="btn btn--sm"
                disabled={!validPartial || phase === "paying"}
                onClick={() => parsedMinor && void pay(parsedMinor)}
                title={validPartial ? "Pay this amount" : "Enter an amount up to the outstanding balance"}
              >
                Pay part
              </button>
            </div>
          </div>
        )}

        <div className="mt-6 rule-t pt-4">
          <div className="eyebrow mb-2">{invoice.work_summary ?? "What this covers"}</div>
          <ul className="space-y-1.5">
            {invoice.lines.map((line, index) => (
              <li key={index} className="flex items-baseline justify-between gap-4">
                <span className="text-[0.8125rem] leading-snug">{line.description}</span>
                <span className="num shrink-0 text-[0.8125rem]">{line.amount}</span>
              </li>
            ))}
          </ul>
          {invoice.memo && invoice.memo !== invoice.work_summary && (
            <p className="mt-3 text-[0.8125rem] leading-relaxed text-[var(--color-ink-2)]">{invoice.memo}</p>
          )}
        </div>

        {invoice.payments.length > 0 && (
          <div className="mt-5 rule-t pt-4">
            <div className="eyebrow mb-2">Receipts</div>
            <ul className="space-y-1.5">
              {invoice.payments.map((payment, index) => (
                <li key={index} className="flex items-baseline justify-between gap-3">
                  <span className="mono-xs">{longDate(payment.at)}</span>
                  <span className="num text-[0.8125rem] text-[var(--color-settled)]">+{payment.amount}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      <section className="mt-10">
        <div className="rule-heavy-b mb-4 flex items-baseline justify-between gap-3 pb-1.5">
          <span className="eyebrow">Conversation</span>
          <span className="mono-xs">{invoicer.name}&rsquo;s assistant answers here</span>
        </div>

        {messages.length === 0 ? (
          <p className="text-[0.8125rem] text-[var(--color-ink-3)]">
            No messages yet — but you can ask a question or propose different payment terms below.
          </p>
        ) : (
          <ol className="space-y-4">
            {messages.map((entry, index) => (
              <li key={index} className={`letter ${entry.author === "client" ? "letter--client" : "letter--agent"}`}>
                <div className="mb-1.5 flex items-baseline justify-between gap-3">
                  <span className="eyebrow">{entry.author === "client" ? "You" : invoicer.name}</span>
                  <span className="mono-xs">{relative(entry.at)}</span>
                </div>
                {entry.subject && <div className="mb-1 text-[0.8125rem] font-medium">{entry.subject}</div>}
                <LetterBody>{entry.body}</LetterBody>
              </li>
            ))}
          </ol>
        )}

        <div className="mt-5">
          <textarea
            className="field min-h-[5.5rem] resize-y"
            placeholder="Ask a question, or propose a plan — e.g. “could we do half now and the rest in two weeks?”"
            value={message}
            onChange={(event) => setMessage(event.target.value)}
          />
          <div className="mt-2 flex items-center justify-between gap-3">
            <span className="mono-xs">
              {invoicer.name}&rsquo;s assistant replies here, in their voice.
            </span>
            <button className="btn btn--solid btn--sm" onClick={() => void sendMessage()} disabled={sending}>
              {sending ? "sending…" : "Send"}
            </button>
          </div>
          {note && <p className="mono-xs mt-2.5 rule-t pt-2.5">{note}</p>}
        </div>
      </section>

      <footer className="mt-12 rule-heavy-t pt-4 text-center">
        <p className="mono-xs leading-relaxed">
          Secured by a signed, expiring link — no account needed. Payments settle through PayPal.
        </p>
        <p className="mono-xs mt-1.5">
          {simulated_payments
            ? "Simulation mode: this deployment does not move real money."
            : "Payments are processed by PayPal; you will see PayPal's own confirmation."}
        </p>
      </footer>
    </div>
  );
}
