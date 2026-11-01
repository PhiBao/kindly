"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { SectionLabel } from "@/components/primitives";
import { api, type Bootstrap } from "@/lib/api-client";
import { useBootstrap } from "@/lib/use-bootstrap";

/**
 * The smallest honest activation flow: add a client, add an invoice, send it.
 * No canned data — the invoice is created in PayPal (or the simulator) and the
 * client gets a real portal link.
 */
export default function NewInvoicePage() {
  const router = useRouter();
  const { data } = useBootstrap();
  const [clients, setClients] = useState<Bootstrap["clients"]>([]);
  const [step, setStep] = useState<"client" | "invoice">("client");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const [client, setClient] = useState({ name: "", company: "", email: "", withCounterpartyAgent: false });
  const [invoice, setInvoice] = useState({
    clientId: "",
    number: "",
    description: "",
    amount: "",
    dueDate: new Date(Date.now() + 14 * 86400000).toISOString().slice(0, 10),
    send: true,
  });

  useEffect(() => {
    if (data) {
      setClients(data.clients);
      if (data.clients[0] && !invoice.clientId) {
        setInvoice((current) => ({ ...current, clientId: data.clients[0].id }));
      }
    }
  }, [data]); // eslint-disable-line react-hooks/exhaustive-deps

  const createClient = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setNote(null);
    try {
      const created = await api.createClient({
        name: client.name.trim(),
        email: client.email.trim(),
        company: client.company.trim() || undefined,
        with_counterparty_agent: client.withCounterpartyAgent,
      });
      setNote(`Added ${created.display_name}.`);
      setInvoice((current) => ({ ...current, clientId: created.client_id }));
      setStep("invoice");
      const list = await api.bootstrap();
      setClients(list.clients);
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(false);
    }
  };

  const createInvoice = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setNote(null);
    try {
      const amountMinor = Math.round(Number(invoice.amount.replace(/[^0-9.]/g, "")) * 100);
      const result = (await api.createInvoice({
        client_id: invoice.clientId,
        number: invoice.number.trim(),
        due_date: invoice.dueDate,
        work_summary: invoice.description.trim(),
        send: invoice.send,
        lines: [
          {
            description: invoice.description.trim(),
            quantity: 1,
            unit_amount_minor: amountMinor,
          },
        ],
      })) as { invoice_id: string };
      router.push(`/i/${result.invoice_id}`);
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
      setBusy(false);
    }
  };

  return (
    <div className="mx-auto w-full max-w-[880px] px-5 pb-24 pt-8 sm:px-8">
      <header className="rule-heavy-b pb-4">
        <Link href="/" className="eyebrow hover:text-[var(--color-ink)]">
          ← the desk
        </Link>
        <h1 className="display mt-2 text-[2rem] leading-none">Add an invoice</h1>
        <p className="mt-2 max-w-[60ch] text-[0.875rem] leading-relaxed text-[var(--color-ink-2)]">
          Two steps: the client, then the invoice. Kindly creates it in PayPal and gives you a signed
          link to send — no seeded data, no demo shortcut.
        </p>
      </header>

      <div className="mt-7 grid gap-3 sm:grid-cols-2">
        <button
          className={`paper-edge p-3 text-left ${step === "client" ? "outline outline-1 outline-[var(--color-ink)]" : ""}`}
          onClick={() => setStep("client")}
        >
          <div className="eyebrow">Step 1</div>
          <div className="mt-1 text-[0.9375rem] font-medium">The client</div>
          <p className="mt-1 text-[0.75rem] text-[var(--color-ink-3)]">
            Who owes you, and whether their side has an agent to negotiate with.
          </p>
        </button>
        <button
          className={`paper-edge p-3 text-left ${step === "invoice" ? "outline outline-1 outline-[var(--color-ink)]" : ""}`}
          onClick={() => setStep("invoice")}
          disabled={clients.length === 0}
        >
          <div className="eyebrow">Step 2</div>
          <div className="mt-1 text-[0.9375rem] font-medium">The invoice</div>
          <p className="mt-1 text-[0.75rem] text-[var(--color-ink-3)]">
            Amount, work, due date. Kindly mirrors it into PayPal.
          </p>
        </button>
      </div>

      {note && <p className="mono-xs mt-4 rule-b pb-2">{note}</p>}

      {step === "client" ? (
        <form className="mt-8 max-w-[560px]" onSubmit={createClient}>
          <SectionLabel>New client</SectionLabel>
          <label className="mt-3 block">
            <span className="eyebrow">Contact name</span>
            <input
              className="field mt-1"
              required
              value={client.name}
              onChange={(event) => setClient({ ...client, name: event.target.value })}
              placeholder="Sam Chen"
            />
          </label>
          <label className="mt-3 block">
            <span className="eyebrow">Company (optional)</span>
            <input
              className="field mt-1"
              value={client.company}
              onChange={(event) => setClient({ ...client, company: event.target.value })}
              placeholder="Acme Studio"
            />
          </label>
          <label className="mt-3 block">
            <span className="eyebrow">Email for the invoice</span>
            <input
              className="field mt-1"
              required
              type="email"
              value={client.email}
              onChange={(event) => setClient({ ...client, email: event.target.value })}
              placeholder="ap@acme.test"
            />
          </label>
          <label className="mt-4 flex items-start gap-2.5 text-[0.8125rem]">
            <input
              type="checkbox"
              className="mt-1"
              checked={client.withCounterpartyAgent}
              onChange={(event) => setClient({ ...client, withCounterpartyAgent: event.target.checked })}
            />
            <span>
              Their side has an accounts-payable agent
              <span className="mono-xs mt-1 block">
                Kindly will negotiate with it machine-to-machine instead of only sending letters.
              </span>
            </span>
          </label>
          <div className="mt-5 flex items-center gap-3">
            <button className="btn btn--solid" type="submit" disabled={busy}>
              {busy ? "saving…" : "Add client"}
            </button>
            {clients.length > 0 && (
              <button className="btn btn--ghost" type="button" onClick={() => setStep("invoice")}>
                skip — use an existing client
              </button>
            )}
          </div>
        </form>
      ) : (
        <form className="mt-8 max-w-[560px]" onSubmit={createInvoice}>
          <SectionLabel>New invoice</SectionLabel>
          <label className="mt-3 block">
            <span className="eyebrow">Client</span>
            <select
              className="field mt-1"
              value={invoice.clientId}
              onChange={(event) => setInvoice({ ...invoice, clientId: event.target.value })}
              required
            >
              {clients.map((option) => (
                <option key={option.id} value={option.id}>
                  {option.display_name}
                </option>
              ))}
            </select>
          </label>
          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="eyebrow">Invoice number</span>
              <input
                className="field mt-1"
                required
                value={invoice.number}
                onChange={(event) => setInvoice({ ...invoice, number: event.target.value })}
                placeholder="1064"
              />
            </label>
            <label className="block">
              <span className="eyebrow">Amount (USD)</span>
              <input
                className="field num mt-1"
                required
                value={invoice.amount}
                onChange={(event) => setInvoice({ ...invoice, amount: event.target.value })}
                placeholder="1,800.00"
              />
            </label>
          </div>
          <label className="mt-3 block">
            <span className="eyebrow">What the work was</span>
            <input
              className="field mt-1"
              required
              value={invoice.description}
              onChange={(event) => setInvoice({ ...invoice, description: event.target.value })}
              placeholder="Brand identity sprint — logo suite, type system"
            />
          </label>
          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <label className="block">
              <span className="eyebrow">Due date</span>
              <input
                className="field mt-1"
                required
                type="date"
                value={invoice.dueDate}
                onChange={(event) => setInvoice({ ...invoice, dueDate: event.target.value })}
              />
            </label>
            <label className="flex items-end gap-2.5 pb-2 text-[0.8125rem]">
              <input
                type="checkbox"
                checked={invoice.send}
                onChange={(event) => setInvoice({ ...invoice, send: event.target.checked })}
              />
              <span>Send it through PayPal now</span>
            </label>
          </div>
          <div className="mt-5 flex items-center gap-3">
            <button className="btn btn--solid" type="submit" disabled={busy}>
              {busy ? "creating…" : "Create invoice"}
            </button>
            <button className="btn btn--ghost" type="button" onClick={() => setStep("client")}>
              back
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
