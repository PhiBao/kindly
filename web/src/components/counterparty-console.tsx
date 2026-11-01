"use client";

import { useEffect, useState } from "react";
import { Badge } from "@/components/primitives";
import { api } from "@/lib/api-client";
import { money, weekdayList } from "@/lib/format";

/**
 * The client's accounts-payable policy, editable by the judge.
 *
 * This is the toucher: change Acme's release limit and re-run the negotiation,
 * and the outcome changes — because the counterparty agent is a real agent with
 * its own mandate, not a script.
 */
export function CounterpartyConsole({
  clientId,
  onChanged,
}: {
  clientId: string;
  onChanged?: () => void;
}) {
  const [policy, setPolicy] = useState<{
    releaseMinor: number;
    installments: number;
    weekdays: number[];
    discountBps: number;
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const data = await api.counterparty(clientId);
        const stored = data.policy as
          | { max_unapproved_cash_minor?: number; max_installments?: number; payment_run_weekdays?: number[]; discount_ask_bps?: number }
          | null;
        if (cancelled) return;
        setPolicy({
          releaseMinor: stored?.max_unapproved_cash_minor ?? data.defaults.max_unapproved_cash_minor,
          installments: stored?.max_installments ?? data.defaults.max_installments,
          weekdays: stored?.payment_run_weekdays ?? data.defaults.payment_run_weekdays,
          discountBps: stored?.discount_ask_bps ?? data.defaults.discount_ask_bps,
        });
      } catch (cause) {
        if (!cancelled) setNote(cause instanceof Error ? cause.message : String(cause));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [clientId]);

  if (!policy) {
    return <p className="mono-xs">Loading their policy…</p>;
  }

  const save = async () => {
    setSaving(true);
    setNote(null);
    try {
      await api.setCounterparty(clientId, {
        max_unapproved_cash_minor: policy.releaseMinor,
        max_installments: policy.installments,
        payment_run_weekdays: policy.weekdays,
        discount_ask_bps: policy.discountBps,
      });
      setNote("Their policy saved. Start a new negotiation to see the difference.");
      onChanged?.();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  const presets = [60_000, 120_000, 200_000, 600_000];

  return (
    <div className="paper-edge p-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <div className="eyebrow">Their policy — you are the judge here</div>
          <p className="mt-1 text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
            The other side is an agent with its own limits. Change them and re-run the negotiation:
            the terms should move, and Kindly should still never cross yours.
          </p>
        </div>
        <Badge tone="neutral">{weekdayList(policy.weekdays)}</Badge>
      </div>

      <div className="mt-4 space-y-3">
        <div>
          <div className="eyebrow mb-1.5">Release without a human</div>
          <div className="flex flex-wrap gap-1.5">
            {presets.map((preset) => (
              <button
                key={preset}
                onClick={() => setPolicy({ ...policy, releaseMinor: preset })}
                className={`num border px-2 py-1 text-[0.75rem] ${
                  policy.releaseMinor === preset
                    ? "border-[var(--color-ink)] bg-[var(--color-ink)] text-[var(--color-paper-2)]"
                    : "border-[var(--color-rule-2)]"
                }`}
              >
                {money(preset)}
              </button>
            ))}
          </div>
        </div>

        <div className="flex flex-wrap gap-5">
          <div>
            <div className="eyebrow mb-1.5">Max instalments</div>
            <div className="flex gap-1.5">
              {[1, 2, 3, 4].map((count) => (
                <button
                  key={count}
                  onClick={() => setPolicy({ ...policy, installments: count })}
                  className={`num border px-2.5 py-1 text-[0.75rem] ${
                    policy.installments === count
                      ? "border-[var(--color-ink)] bg-[var(--color-ink)] text-[var(--color-paper-2)]"
                      : "border-[var(--color-rule-2)]"
                  }`}
                >
                  {count}
                </button>
              ))}
            </div>
          </div>
          <div>
            <div className="eyebrow mb-1.5">Discount they ask for</div>
            <div className="flex gap-1.5">
              {[0, 300, 500, 1000].map((bps) => (
                <button
                  key={bps}
                  onClick={() => setPolicy({ ...policy, discountBps: bps })}
                  className={`num border px-2 py-1 text-[0.75rem] ${
                    policy.discountBps === bps
                      ? "border-[var(--color-ink)] bg-[var(--color-ink)] text-[var(--color-paper-2)]"
                      : "border-[var(--color-rule-2)]"
                  }`}
                >
                  {bps / 100}%
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>

      <div className="mt-3 flex items-center gap-2">
        <button className="btn btn--sm" onClick={() => void save()} disabled={saving}>
          {saving ? "saving…" : "Save their policy"}
        </button>
        <span className="mono-xs">
          release {money(policy.releaseMinor)} · {policy.installments} instalment(s) · {policy.discountBps / 100}% ask
        </span>
      </div>
      {note && <p className="mono-xs mt-2 rule-t pt-2">{note}</p>}
    </div>
  );
}
