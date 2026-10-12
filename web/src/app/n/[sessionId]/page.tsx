"use client";

import Link from "next/link";
import { useParams } from "next/navigation";
import { useState } from "react";
import { Badge, Stamp } from "@/components/primitives";
import { api, type KatpEntry } from "@/lib/api-client";
import { describeTerms, longDate, money, relative, weekdayList } from "@/lib/format";
import { useBootstrap } from "@/lib/use-bootstrap";

const TYPE_TONE: Record<string, string> = {
  hello: "neutral",
  offer: "amber",
  counter: "amber",
  accept: "settled",
  reject: "stamp",
  settle: "settled",
  authorise: "settled",
  info: "neutral",
};

export default function NegotiationRoomPage() {
  const params = useParams<{ sessionId: string }>();
  const { data, refresh } = useBootstrap();
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const session = data?.sessions.find((candidate) => candidate.id === params.sessionId) ?? null;
  const invoice = session ? data?.invoices.find((candidate) => candidate.id === session.invoice_id) : null;
  const instalments = (data?.invoices ?? []).filter(
    (candidate) => session && candidate.number.startsWith(`${invoice?.number}-P`),
  );
  const mandate = data?.mandate;

  const run = async (name: string, action: () => Promise<unknown>) => {
    setBusy(name);
    setNote(null);
    try {
      const result = await action();
      const status = (result as { status?: string } | undefined)?.status;
      if (status) setNote(`Session is now: ${status.replace(/_/g, " ")}.`);
      await refresh();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!session) {
    return (
      <div className="mx-auto max-w-[900px] px-5 py-16">
        <div className="mono-xs">
          {data ? "That negotiation is no longer open (agent keys are per instance — start a new one from the desk)." : "Loading…"}
        </div>
        <Link className="btn mt-4 inline-flex" href="/">
          back to the desk
        </Link>
      </div>
    );
  }

  return (
    <div className="mx-auto w-full max-w-[1180px] px-5 pb-28 sm:px-8">
      <header className="rule-heavy-b flex flex-wrap items-end justify-between gap-4 pb-4 pt-7">
        <div>
          <Link href={invoice ? `/i/${invoice.id}` : "/"} className="eyebrow hover:text-[var(--color-ink)]">
            ← {invoice ? `invoice ${invoice.number}` : "the desk"}
          </Link>
          <h1 className="display mt-2 text-[2rem] leading-none">Negotiation room</h1>
          <p className="mt-1.5 text-[0.875rem] text-[var(--color-ink-2)]">
            Two agents, one outstanding invoice, signed terms — and a human on each side where it matters.
          </p>
        </div>
        <div className="flex items-center gap-3 pr-1">
          <span className="mono-xs">
            {session.rounds} round{session.rounds === 1 ? "" : "s"} · {session.transcript.length} messages
          </span>
          <RoomStamp status={session.status} />
        </div>
      </header>

      <div className="mt-7 grid gap-4 lg:grid-cols-2">
        <AgentCard
          side="seller"
          title={data?.owner.business ?? "You"}
          subtitle="Kindly · collections agent"
          lines={mandate?.digest ?? []}
          publicKey={session.transcript[0]?.message ? undefined : undefined}
          verified={session.transcript.every((entry) => entry.verified)}
          accent="ink"
        />
        <AgentCard
          side="buyer"
          title={session.buyer.company}
          subtitle={`${session.buyer.contact_name} · their agent`}
          lines={dedupePolicy(session.buyer.policy, session.buyer)}
          publicKey={session.buyer.public_key}
          verified
          accent="settled"
        />
      </div>

      <section className="mt-9">
        <div className="rule-heavy-b mb-4 flex items-baseline justify-between gap-4 pb-1.5">
          <span className="eyebrow">Transcript</span>
          <span className="mono-xs">
            KATP v0.1 · Ed25519 signatures · every envelope verified on receipt
          </span>
        </div>
        <ol>
          {session.transcript.map((entry, index) => (
            <TranscriptLine key={index} entry={entry} />
          ))}
        </ol>
      </section>

      <section className="mt-8 paper-edge p-5">
        <div className="eyebrow">Decision</div>
        <div className="mt-3 flex flex-wrap items-center gap-2.5">
          {session.status === "negotiating" && (
            <button
              className="btn btn--solid"
              disabled={busy !== null}
              onClick={() => void run("step", () => api.a2aStep(session.id))}
            >
              {busy === "step" ? "exchanging…" : "Advance the negotiation"}
            </button>
          )}

          {session.status === "awaiting_owner_approval" && (
            <>
              <span className="text-[0.8125rem] text-[var(--color-stamp)]">
                Their counter breaks a limit you set. This is your call, not the agent&rsquo;s.
              </span>
              <button
                className="btn btn--stamp"
                disabled={busy !== null}
                onClick={() => void run("approve", () => api.a2aOwnerDecision(session.id, true))}
              >
                Approve the exception
              </button>
              <button
                className="btn"
                disabled={busy !== null}
                onClick={() => void run("hold", () => api.a2aOwnerDecision(session.id, false))}
              >
                Hold the line
              </button>
            </>
          )}

          {session.status === "awaiting_buyer_approval" && (
            <>
              <span className="text-[0.8125rem] text-[var(--color-ink-2)]">
                {session.buyer.company}&rsquo;s agent cannot release that much cash without a human. Step into their
                side to see it land.
              </span>
              <button
                className="btn btn--solid"
                disabled={busy !== null}
                onClick={() => void run("buyer", () => api.a2aBuyerApproval(session.id))}
              >
                Their finance lead approves
              </button>
            </>
          )}

          {(session.status === "agreed" || session.status === "plan_issued") && (
            <>
              <span className="text-[0.8125rem] text-[var(--color-settled)]">
                Agreed inside both mandates. Kindly issued the deferred part as real PayPal invoices.
              </span>
              <button
                className="btn btn--solid"
                disabled={busy !== null}
                onClick={() =>
                  void run("instalment", () => api.scenario("plan_instalment_paid"))
                }
              >
                The client pays an instalment
              </button>
            </>
          )}

          {session.status === "settled" && (
            <div className="flex flex-wrap items-center gap-4">
              <Stamp label="Settled" tone="settled" live />
              <span className="text-[0.8125rem] text-[var(--color-ink-2)]">
                Payment recorded against the plan and attributed in the ledger.
              </span>
              <Link className="btn btn--sm" href={invoice ? `/i/${invoice.id}` : "/"}>
                Open the invoice
              </Link>
            </div>
          )}

          {(session.status === "stalled" || session.status === "rejected") && (
            <div className="flex flex-wrap items-center gap-4">
              <span className="text-[0.8125rem] text-[var(--color-ink-2)]">
                {session.status === "rejected"
                  ? "Their agent declined. Kindly has stopped contacting them and flagged it for you."
                  : "Round limit reached without agreement. Kindly stopped rather than conceding outside your limits."}
              </span>
              <Link className="btn btn--sm" href="/">
                Back to the desk
              </Link>
            </div>
          )}
        </div>

        {instalments.length > 0 && (
          <div className="mt-4 rule-t pt-3">
            <div className="eyebrow mb-2">Issued from this agreement</div>
            <ul className="space-y-1.5">
              {instalments.map((instalment) => (
                <li key={instalment.id} className="flex items-baseline justify-between gap-3">
                  <span className="num text-[0.8125rem]">{instalment.number}</span>
                  <span className="mono-xs">due {longDate(instalment.due_date)}</span>
                  <span className="num text-[0.8125rem]">{instalment.total}</span>
                  <Badge tone={instalment.status === "paid" ? "settled" : "neutral"}>
                    {instalment.status_label.toLowerCase()}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        )}

        {note && <p className="mono-xs mt-3 rule-t pt-2.5">{note}</p>}
      </section>
    </div>
  );
}

function AgentCard({
  title,
  subtitle,
  lines,
  publicKey,
  verified,
  accent,
}: {
  side: string;
  title: string;
  subtitle: string;
  lines: string[];
  publicKey?: string;
  verified: boolean;
  accent: "ink" | "settled";
}) {
  return (
    <section className="paper-edge p-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="eyebrow">{subtitle}</div>
          <div className="display mt-1 text-[1.35rem] leading-tight">{title}</div>
        </div>
        <Badge tone={accent === "settled" ? "settled" : "neutral"}>{verified ? "signed" : "unsigned"}</Badge>
      </div>
      <ul className="mt-3 space-y-1.5">
        {lines.map((line) => (
          <li key={line} className="flex gap-2 text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
            <span aria-hidden className="text-[var(--color-rule-2)]">
              —
            </span>
            <span>{line}</span>
          </li>
        ))}
      </ul>
      {publicKey && (
        <p className="mono-xs mt-3 truncate" title={publicKey}>
          key {publicKey.slice(0, 24)}…
        </p>
      )}
    </section>
  );
}

function TranscriptLine({ entry }: { entry: KatpEntry }) {
  const message = entry.message as {
    type?: string;
    note?: string;
    terms?: Record<string, unknown>;
    reason?: string;
    reference?: string;
    amount_minor?: number;
    currency?: string;
  };
  const type = message.type ?? "info";
  const isBuyer = entry.speaker === "client_agent";

  return (
    <li className="katp-line rule-b">
      <span className="katp-rail">
        <span className={`katp-node ${isBuyer ? "katp-node--buyer" : ""}`} />
      </span>
      <div>
        <div className="flex flex-wrap items-baseline gap-2.5">
          <span className="text-[0.8125rem] font-medium">{entry.label}</span>
          <Badge tone={(TYPE_TONE[type] ?? "neutral") as "neutral" | "amber" | "settled" | "stamp"}>
            {type}
          </Badge>
          {entry.verified ? <Badge tone="settled">verified</Badge> : <Badge tone="stamp">unverified</Badge>}
          <span className="mono-xs">{relative(entry.at)}</span>
        </div>

        {message.terms && (
          <p className="num mt-1.5 text-[0.8125rem]">{describeTerms(message.terms as never)}</p>
        )}
        {message.note && <p className="letter-body mt-1.5 text-[0.9375rem]">{message.note}</p>}
        {message.reason && <p className="text-[0.8125rem] text-[var(--color-stamp)]">{message.reason}</p>}
        {message.reference && (
          <p className="mono-xs mt-1">
            PayPal reference {message.reference}
            {message.amount_minor ? ` · ${money(message.amount_minor, message.currency ?? "USD")}` : ""}
          </p>
        )}
        <p className="mono-xs mt-1.5">{entry.note}</p>
      </div>
    </li>
  );
}

/**
 * The client's agent publishes its cash policy in its own words. Where the
 * policy already covers a fact, we do not repeat it as a generated line.
 */
function dedupePolicy(policy: string[], buyer: { max_unapproved_cash_minor: number; max_installments: number }): string[] {
  const lines = [...policy];
  const mentionsRelease = policy.some((line) => /approv|release|sign-?off/i.test(line));
  const mentionsRuns = policy.some((line) => /run|friday|fridays|schedule/i.test(line));
  if (!mentionsRelease) lines.push(`Releases above ${money(buyer.max_unapproved_cash_minor)} need a human.`);
  if (!mentionsRuns) lines.push(`Payment runs: ${weekdayList((buyer as { payment_run_weekdays?: number[] }).payment_run_weekdays ?? [])}.`);
  lines.push(`Up to ${buyer.max_installments} instalments.`);
  return lines;
}

function RoomStamp({ status }: { status: string }) {
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


