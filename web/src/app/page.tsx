"use client";

import Link from "next/link";
import { AgentStream, ApprovalInbox } from "@/components/agent-stream";
import { DeskShell } from "@/components/desk-shell";
import { MandateCard } from "@/components/mandate-card";
import { Badge, Metric, SectionLabel, Stamp } from "@/components/primitives";
import { ProofPanel } from "@/components/proof-panel";
import { ReceivablesLedger } from "@/components/receivables";
import { shortDate } from "@/lib/format";
import { useBootstrap } from "@/lib/use-bootstrap";

export default function DeskPage() {
  const { data, error, loading, refresh } = useBootstrap();

  const invoiceNumbers = Object.fromEntries((data?.invoices ?? []).map((invoice) => [invoice.id, invoice.number]));
  const sessions = data?.sessions ?? [];
  const activeSessions = sessions.filter((session) => session.status !== "rejected");

  return (
    <DeskShell data={data} onRefresh={refresh}>
      {error && (
        <div className="paper-edge mb-6 border-[var(--color-stamp)] p-4">
          <div className="eyebrow text-[var(--color-stamp)]">The server is not answering</div>
          <p className="mt-1 text-[0.8125rem]">{error}</p>
          <p className="mono-xs mt-1.5">
            Start it with <code>scripts/dev-server.sh</code> (simulation mode needs no credentials).
          </p>
        </div>
      )}

      {loading && !data && <div className="mono-xs">Loading the desk…</div>}

      {data && (
        <>
          <section className="rule-heavy-b pb-5">
            <div className="flex flex-wrap items-end justify-between gap-6">
              <h1 className="display max-w-[26ch] text-[2.1rem] leading-[1.05] sm:text-[2.6rem]">
                You do the work.
                <br />
                Kindly does the asking.
              </h1>
              <p className="max-w-[42ch] text-[0.875rem] leading-relaxed text-[var(--color-ink-2)]">
                Undone invoicing is not an admin problem, it is an emotional one. Kindly writes in your voice,
                negotiates what your clients ask for, and settles it through PayPal — inside limits you set and can
                see enforced.
              </p>
            </div>

            <div className="mt-7 grid grid-cols-2 gap-6 sm:grid-cols-4">
              <Metric label="Outstanding" value={data.stats.outstanding} hint={`${data.stats.invoices_open} open invoices`} />
              <Metric
                label="Overdue"
                value={String(data.stats.overdue_count)}
                hint="past their due date"
                tone={data.stats.overdue_count > 0 ? "stamp" : "ink"}
              />
              <Metric label="Collected" value={data.stats.collected} hint="all time, through PayPal" tone="settled" />
              <Metric
                label="Days saved"
                value={data.stats.average_days_saved === null ? "—" : `${data.stats.average_days_saved}`}
                hint="average, vs each client's own habit"
              />
            </div>
          </section>

          <div className="mt-9 grid gap-10 lg:grid-cols-[minmax(0,1.55fr)_minmax(0,1fr)]">
            <div className="space-y-10">
              <ReceivablesLedger invoices={data.invoices} />
              <AgentStream events={data.ledger} invoiceNumbers={invoiceNumbers} />
            </div>

            <div className="space-y-10">
              <MandateCard mandate={data.mandate} onSaved={refresh} />

              <ApprovalInbox approvals={data.approvals} invoiceNumbers={invoiceNumbers} onDone={refresh} />

              <section>
                <SectionLabel aside={<span className="mono-xs">{activeSessions.length}</span>}>
                  Negotiations
                </SectionLabel>
                {activeSessions.length === 0 ? (
                  <p className="text-[0.8125rem] text-[var(--color-ink-3)]">
                    No agent-to-agent talks yet. Run the first scenario to open one.
                  </p>
                ) : (
                  <ul className="space-y-3">
                    {activeSessions.map((session) => (
                      <li key={session.id} className="paper-edge p-3.5">
                        <div className="flex items-start justify-between gap-3">
                          <div>
                            <div className="text-[0.875rem] font-medium">{session.buyer.company}</div>
                            <div className="mono-xs mt-0.5">
                              invoice {invoiceNumbers[session.invoice_id] ?? "—"} · {session.rounds} round
                              {session.rounds === 1 ? "" : "s"} · {session.transcript.length} signed messages
                            </div>
                          </div>
                          <SessionStamp status={session.status} />
                        </div>
                        <p className="mt-2 line-clamp-2 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
                          {session.transcript[session.transcript.length - 1]?.note}
                        </p>
                        <Link className="btn btn--sm mt-3 inline-flex" href={`/n/${session.id}`}>
                          Open the room
                        </Link>
                      </li>
                    ))}
                  </ul>
                )}
              </section>

              <ProofPanel
                attributions={data.attributions}
                paypalEvents={data.paypal_events}
                ledger={data.ledger}
              />

              <section>
                <SectionLabel aside={<span className="mono-xs">live</span>}>Integration</SectionLabel>
                <ul className="space-y-1.5">
                  <li className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                    <span className="text-[0.8125rem]">PayPal</span>
                    <Badge tone={data.owner.paypal_mode_raw === "live" ? "settled" : "amber"}>
                      {data.owner.paypal_mode_raw === "live" ? "sandbox API" : "simulator"}
                    </Badge>
                  </li>
                  <li className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                    <span className="text-[0.8125rem]">Language model</span>
                    <Badge tone={data.environment.model_enabled ? "settled" : "neutral"}>
                      {data.environment.model_enabled ? "connected" : "template fallback"}
                    </Badge>
                  </li>
                  <li className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                    <span className="text-[0.8125rem]">Invoicing identity</span>
                    <span className="mono-xs">{data.owner.invoicer_email}</span>
                  </li>
                  <li className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                    <span className="text-[0.8125rem]">Contact window</span>
                    <span className="mono-xs">{data.mandate.contact_window}</span>
                  </li>
                  <li className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                    <span className="text-[0.8125rem]">Agent key</span>
                    <span className="mono-xs">Ed25519 · per instance</span>
                  </li>
                </ul>
                <p className="mono-xs mt-3 leading-relaxed">
                  Mode: {data.environment.model_enabled ? data.environment.model : "deterministic templates"} · API{" "}
                  {data.environment.paypal_base_url.replace("https://", "")}
                </p>
              </section>
            </div>
          </div>

          <p className="mono-xs mt-12 leading-relaxed text-[var(--color-ink-3)]">
            Open a client link from any invoice desk to see exactly what your client sees — and pay as them.{" "}
            {data.paypal_events.length > 0 && `Latest PayPal event ${shortDate(data.paypal_events[0].received_at)}.`}
          </p>
        </>
      )}
    </DeskShell>
  );
}

function SessionStamp({ status }: { status: string }) {
  switch (status) {
    case "settled":
      return <Stamp label="Settled" tone="settled" />;
    case "plan_issued":
      return <Stamp label="Plan issued" tone="settled" />;
    case "agreed":
      return <Stamp label="Agreed" tone="settled" />;
    case "awaiting_owner_approval":
      return <Stamp label="Needs you" />;
    case "awaiting_buyer_approval":
      return <Stamp label="Their approval" tone="quiet" />;
    case "stalled":
      return <Stamp label="Stalled" tone="quiet" />;
    case "rejected":
      return <Stamp label="Declined" tone="quiet" />;
    default:
      return <Stamp label="In talks" tone="quiet" />;
  }
}
