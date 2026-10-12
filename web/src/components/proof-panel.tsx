"use client";

import { Badge, Empty, SectionLabel } from "@/components/primitives";
import type { Attribution, LedgerEvent, PaypalEvent } from "@/lib/api-client";
import { longDate, relative, shortDate } from "@/lib/format";

const CAUSE_COPY: Record<string, string> = {
  plan_settled: "the plan Kindly negotiated",
  reminder_led: "a reminder that landed",
  promise_kept: "a promise that came true",
  organic: "the client paying on their own",
  unknown: "not enough signal",
};

export function ProofPanel({
  attributions,
  paypalEvents,
  ledger,
}: {
  attributions: Attribution[];
  paypalEvents: PaypalEvent[];
  ledger: LedgerEvent[];
}) {
  const security = ledger.filter((event) => event.kind === "security_event");
  const blocks = ledger.filter((event) => event.kind === "policy_blocked");

  return (
    <section>
      <SectionLabel aside={<span className="mono-xs">no credit taken for luck</span>}>
        What got you paid
      </SectionLabel>

      {attributions.length === 0 ? (
        <Empty>Settle an invoice and the cause will be recorded here — including when it wasn&rsquo;t the agent.</Empty>
      ) : (
        <ul className="space-y-2.5">
          {attributions.slice(0, 5).map((attribution, index) => (
            <li key={`${attribution.invoice_id}-${index}`} className="rule-b pb-2.5">
              <div className="flex items-baseline justify-between gap-3">
                <span className="text-[0.875rem]">
                  {attribution.client_name} · invoice <span className="num">{attribution.invoice_number}</span>
                </span>
                <Badge tone={attribution.kind === "organic" ? "neutral" : "settled"}>
                  {attribution.kind.replace(/_/g, " ")}
                </Badge>
              </div>
              <p className="mt-1 text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">{attribution.rationale}</p>
              <p className="mono-xs mt-1">
                {attribution.days_to_paid !== null && `${attribution.days_to_paid} days to payment`}
                {attribution.days_saved !== null && attribution.days_saved > 0 && ` · ${attribution.days_saved} days earlier than this client's average`}
                {" · confidence "}
                {attribution.confidence}
              </p>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-6">
        <SectionLabel aside={<span className="mono-xs">{paypalEvents.length} recent</span>}>
          PayPal deliveries
        </SectionLabel>
        {paypalEvents.length === 0 ? (
          <Empty>No webhooks yet. Pay an invoice to see one arrive.</Empty>
        ) : (
          <ul className="space-y-1.5">
            {paypalEvents.map((event) => (
              <li key={event.event_id} className="flex items-baseline justify-between gap-3 rule-b py-1.5">
                <span className="min-w-0">
                  <span className="num block truncate text-[0.75rem]">{event.event_type}</span>
                  <span className="mono-xs">{relative(event.received_at)}</span>
                </span>
                <span className="flex shrink-0 items-center gap-2">
                  <Badge tone={event.verified ? "settled" : "stamp"}>{event.verified ? "verified" : "unverified"}</Badge>
                  <Badge tone="neutral">{event.status}</Badge>
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="mt-6 grid gap-3 sm:grid-cols-2">
        <div className="rule-t pt-2.5">
          <div className="eyebrow">Limits enforced</div>
          <div className="num mt-1 text-[1.4rem]">{blocks.length}</div>
          <p className="mt-1 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
            Actions the agent wanted to take and your mandate refused.
            {blocks[0] && (
              <>
                {" "}
                Most recently: <em className="not-italic text-[var(--color-stamp)]">{blocks[0].summary}</em>
              </>
            )}
          </p>
        </div>
        <div className="rule-t pt-2.5">
          <div className="eyebrow">Instructions refused</div>
          <div className="num mt-1 text-[1.4rem]">{security.length}</div>
          <p className="mt-1 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
            {security[0] ? security[0].summary : "Client messages are treated as data, never as orders."}
          </p>
        </div>
      </div>
    </section>
  );
}

export function Timeline({ events }: { events: LedgerEvent[] }) {
  return (
    <ol className="space-y-2">
      {events.map((event, index) => (
        <li key={`${event.at}-${index}`} className="grid grid-cols-[6.5rem_1fr] gap-3 rule-b pb-2">
          <span className="mono-xs pt-[0.15rem]">{shortDate(event.at)} {new Date(event.at).getUTCFullYear()}</span>
          <span>
            <span className="text-[0.8125rem] leading-snug">{event.summary}</span>
            <span className="mono-xs mt-0.5 block">
              {event.actor_label} · {longDate(event.at)}
            </span>
          </span>
        </li>
      ))}
    </ol>
  );
}
