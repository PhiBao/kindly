"use client";

import Link from "next/link";
import { Badge, Stamp } from "@/components/primitives";
import type { InvoiceSummary } from "@/lib/api-client";
import { shortDate } from "@/lib/format";

function stateStamp(invoice: InvoiceSummary) {
  if (invoice.status === "paid") return <Stamp label="Paid" tone="settled" />;
  if (invoice.negotiation.stage === "human_review") return <Stamp label="Needs you" />;
  if (invoice.status === "partially_paid") return <Stamp label="Part paid" />;
  if (invoice.status === "overdue" && invoice.days_overdue > 0)
    return <Stamp label={`${invoice.days_overdue}d late`} />;
  if (invoice.negotiation.stage === "terms_proposed") return <Stamp label="Plan offered" tone="quiet" />;
  if (invoice.negotiation.stage === "negotiating") return <Stamp label="In talks" tone="quiet" />;
  if (invoice.negotiation.stage === "awaiting_settlement") return <Stamp label="Agreed" tone="settled" />;
  return <Stamp label="Sent" tone="quiet" />;
}

const STAGE_COPY: Record<string, string> = {
  idle: "ready for a move",
  contacted: "awaiting a reply",
  awaiting_reply: "in conversation",
  negotiating: "negotiating terms",
  terms_proposed: "terms on the table",
  awaiting_settlement: "awaiting payment",
  human_review: "needs you",
  paused: "paused",
  closed_paid: "settled",
  closed_uncollectible: "closed",
};

export function ReceivablesLedger({ invoices }: { invoices: InvoiceSummary[] }) {
  const open = invoices.filter((invoice) => invoice.status !== "paid" && invoice.status !== "cancelled");
  const settled = invoices.filter((invoice) => invoice.status === "paid");

  return (
    <div>
      <div className="rule-heavy-b pb-1.5">
        <div className="flex items-baseline justify-between gap-4">
          <span className="eyebrow">Receivables</span>
          <span className="mono-xs">{open.length} open</span>
        </div>
      </div>

      <div className="rule-b hidden grid-cols-[7rem_1fr_7.5rem_4.5rem_9.5rem] gap-3 py-1.5 sm:grid">
        <span className="eyebrow">Invoice</span>
        <span className="eyebrow">Client &amp; work</span>
        <span className="eyebrow text-right">Outstanding</span>
        <span className="eyebrow">Due</span>
        <span className="eyebrow pl-1">Where it stands</span>
      </div>

      <ul>
        {invoices.map((invoice) => (
          <li key={invoice.id} className="ledger-row rule-b">
            <Link
              href={`/i/${invoice.id}`}
              className="grid grid-cols-[1fr_auto] items-center gap-x-3 gap-y-2 py-3 sm:grid-cols-[7rem_1fr_7.5rem_4.5rem_9.5rem] sm:items-center"
            >
              <span className="num text-[0.8125rem]">{invoice.number}</span>

              <span className="order-3 col-span-2 sm:order-none sm:col-span-1">
                <span className="block text-[0.875rem] font-medium leading-tight">{invoice.client_name}</span>
                <span className="mt-0.5 line-clamp-1 block text-[0.75rem] text-[var(--color-ink-3)]">
                  {invoice.work_summary ?? "—"}
                  {invoice.behavior && invoice.behavior.avg_days_late > 0 && (
                    <> · pays ~{invoice.behavior.avg_days_late}d late</>
                  )}
                </span>
              </span>

              <span className="num justify-self-end text-right text-[0.875rem] sm:justify-self-end">
                {invoice.outstanding}
                {invoice.paid_minor > 0 && invoice.outstanding_minor > 0 && (
                  <span className="mono-xs mt-0.5 block">of {invoice.total}</span>
                )}
              </span>

              <span className="num hidden text-[0.75rem] text-[var(--color-ink-3)] sm:block">
                {shortDate(invoice.due_date)}
              </span>

              <span className="flex items-center gap-2.5 pl-1">
                {stateStamp(invoice)}
                <span className="mono-xs hidden leading-tight lg:block">
                  {STAGE_COPY[invoice.negotiation.stage] ?? invoice.negotiation.stage_label}
                </span>
              </span>
            </Link>
          </li>
        ))}
      </ul>

      {settled.length > 0 && (
        <div className="mt-5">
          <div className="eyebrow mb-2">Settled</div>
          <ul>
            {settled.map((invoice) => (
              <li key={invoice.id} className="rule-b">
                <Link href={`/i/${invoice.id}`} className="flex items-baseline justify-between gap-3 py-1.5">
                  <span className="flex items-baseline gap-2.5">
                    <span className="num text-[0.8125rem]">{invoice.number}</span>
                    <span className="text-[0.8125rem] text-[var(--color-ink-2)]">{invoice.client_name}</span>
                  </span>
                  <span className="flex items-center gap-2.5">
                    <span className="num text-[0.8125rem]">{invoice.total}</span>
                    <Badge tone="settled">paid</Badge>
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
