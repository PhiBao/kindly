"use client";

import { useState } from "react";
import { Empty, SectionLabel } from "@/components/primitives";
import { api, type AgentAction, type LedgerEvent } from "@/lib/api-client";
import { relative } from "@/lib/format";

const TRUST_KINDS = new Set([
  "policy_blocked",
  "policy_approval_requested",
  "policy_approval_granted",
  "policy_approval_rejected",
  "security_event",
  "mandate_updated",
]);

export function AgentStream({ events, invoiceNumbers }: { events: LedgerEvent[]; invoiceNumbers: Record<string, string> }) {
  return (
    <section>
      <SectionLabel aside={<span className="mono-xs">newest first</span>}>The agent&rsquo;s trail</SectionLabel>
      {events.length === 0 ? (
        <Empty>Nothing yet. Run the agent.</Empty>
      ) : (
        <ol className="marquee-fade max-h-[30rem] space-y-0 overflow-y-auto pr-1">
          {events.slice(0, 40).map((event, index) => {
            const trust = event.trust || TRUST_KINDS.has(event.kind);
            const invoiceNumber = event.invoice_id ? invoiceNumbers[event.invoice_id as string] : undefined;
            return (
              <li
                key={`${event.at}-${index}`}
                className={`stream-line flex gap-3 py-2 ${index === 0 ? "" : "rule-t"}`}
              >
                <span className="mono-xs w-[3.5rem] shrink-0 pt-[0.15rem]">{relative(event.at)}</span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-2">
                    {trust && <span className="trust-mark" aria-hidden />}
                    <span className="text-[0.8125rem] leading-snug">{event.summary}</span>
                  </span>
                  <span className="mono-xs mt-0.5 block">
                    {event.actor_label ?? event.actor}
                    {invoiceNumber ? ` · invoice ${invoiceNumber}` : ""}
                  </span>
                </span>
              </li>
            );
          })}
        </ol>
      )}
    </section>
  );
}

export function ApprovalInbox({
  approvals,
  invoiceNumbers,
  onDone,
}: {
  approvals: AgentAction[];
  invoiceNumbers: Record<string, string>;
  onDone: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const decide = async (action: AgentAction, approve: boolean) => {
    setBusy(action.id);
    setNote(null);
    try {
      await api.decide(action.id, approve);
      setNote(approve ? `${action.kind_label} approved and carried out.` : `${action.kind_label} declined.`);
      onDone();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section>
      <SectionLabel aside={<span className="mono-xs">{approvals.length} waiting</span>}>
        Needs your decision
      </SectionLabel>
      {approvals.length === 0 ? (
        <Empty>Nothing is waiting on you. The agent is working inside your limits.</Empty>
      ) : (
        <ul className="space-y-3">
          {approvals.map((action) => {
            const draft = (action as unknown as { draft?: { subject?: string; body?: string } }).draft;
            const reason = action.decision_explanation;
            return (
              <li key={action.id} className="paper-edge p-3.5">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="text-[0.875rem] font-medium">{action.kind_label}</span>
                  <span className="mono-xs">
                    {invoiceNumbers[action.invoice_id] ? `invoice ${invoiceNumbers[action.invoice_id]}` : ""}
                  </span>
                </div>
                <p className="mt-1 text-[0.8125rem] leading-snug text-[var(--color-stamp)]">{reason}</p>
                {action.rationale && action.rationale !== reason && (
                  <p className="mt-1.5 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">{action.rationale}</p>
                )}

                {draft?.body && (
                  <div className="letter letter--agent mt-3">
                    {draft.subject && <div className="eyebrow mb-1.5">{draft.subject}</div>}
                    <p className="letter-body whitespace-pre-wrap">{draft.body}</p>
                  </div>
                )}

                <div className="mt-3 flex items-center gap-2">
                  <button
                    className="btn btn--stamp btn--sm"
                    onClick={() => void decide(action, true)}
                    disabled={busy !== null}
                  >
                    {busy === action.id ? "working…" : "Approve"}
                  </button>
                  <button className="btn btn--sm" onClick={() => void decide(action, false)} disabled={busy !== null}>
                    Decline
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {note && <p className="mono-xs mt-3 rule-t pt-2.5">{note}</p>}
    </section>
  );
}
