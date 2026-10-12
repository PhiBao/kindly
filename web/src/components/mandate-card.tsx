"use client";

import { useState } from "react";
import { Badge } from "@/components/primitives";
import { api, type Mandate } from "@/lib/api-client";
import { bps } from "@/lib/format";

const AUTONOMY: { value: Mandate["autonomy"]; label: string; hint: string }[] = [
  { value: "observe", label: "Observe", hint: "Watch and report only — no contact, ever." },
  { value: "suggest_only", label: "Suggest", hint: "Drafts everything; nothing leaves without you." },
  { value: "act_within_limits", label: "Act", hint: "Acts inside the limits below; asks when a limit is crossed." },
];

const FLOORS = [5000, 6000, 7000, 8000, 10000];

export function MandateCard({ mandate, onSaved }: { mandate: Mandate; onSaved: () => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(mandate);
  const [saving, setSaving] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const save = async () => {
    setSaving(true);
    setNote(null);
    try {
      await api.mandate({
        autonomy: draft.autonomy,
        max_contacts_per_week: draft.max_contacts_per_week,
        min_days_between_contacts: draft.min_days_between_contacts,
        min_upfront_settlement_bps: draft.min_upfront_settlement_bps,
        max_discount_bps: draft.max_discount_bps,
        max_installments: draft.max_installments,
        allow_agent_to_agent: draft.allow_agent_to_agent,
      });
      setNote("Mandate saved. The agent will use it on its next decision.");
      setEditing(false);
      onSaved();
    } catch (cause) {
      setNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section className="paper-edge p-4 sm:p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="eyebrow">Your mandate</div>
          <p className="display mt-1 text-[1.35rem] leading-tight">{mandate.autonomy_label}</p>
          <p className="mt-1 max-w-[46ch] text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
            {mandate.autonomy_description}
          </p>
        </div>
        <div className="shrink-0 text-right">
          <Badge tone="neutral">v{mandate.version}</Badge>
          <button className="btn btn--sm mt-2 block" onClick={() => setEditing((open) => !open)}>
            {editing ? "cancel" : "adjust"}
          </button>
        </div>
      </div>

      {!editing && (
        <ul className="mt-4 space-y-1.5">
          {mandate.digest.map((line) => (
            <li key={line} className="flex gap-2 text-[0.8125rem] leading-snug text-[var(--color-ink-2)]">
              <span aria-hidden className="text-[var(--color-rule-2)]">
                —
              </span>
              <span>{line}</span>
            </li>
          ))}
        </ul>
      )}

      {editing && (
        <div className="mt-4 space-y-3.5">
          <div>
            <div className="eyebrow mb-1.5">Autonomy</div>
            <div className="grid gap-1.5 sm:grid-cols-3">
              {AUTONOMY.map((option) => (
                <button
                  key={option.value}
                  onClick={() => setDraft({ ...draft, autonomy: option.value })}
                  className={`border px-2.5 py-2 text-left transition-colors ${
                    draft.autonomy === option.value
                      ? "border-[var(--color-ink)] bg-[color-mix(in_srgb,var(--color-ink)_6%,transparent)]"
                      : "border-[var(--color-rule-2)] hover:border-[var(--color-ink-3)]"
                  }`}
                >
                  <span className="block text-[0.8125rem] font-medium">{option.label}</span>
                  <span className="mt-0.5 block text-[0.6875rem] leading-snug text-[var(--color-ink-3)]">
                    {option.hint}
                  </span>
                </button>
              ))}
            </div>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div>
              <div className="eyebrow mb-1.5">Never settle below</div>
              <div className="flex flex-wrap gap-1.5">
                {FLOORS.map((floor) => (
                  <button
                    key={floor}
                    onClick={() => setDraft({ ...draft, min_upfront_settlement_bps: floor })}
                    className={`num border px-2 py-1 text-[0.75rem] ${
                      draft.min_upfront_settlement_bps === floor
                        ? "border-[var(--color-ink)] bg-[var(--color-ink)] text-[var(--color-paper-2)]"
                        : "border-[var(--color-rule-2)]"
                    }`}
                  >
                    {bps(floor)}
                  </button>
                ))}
              </div>
            </div>

            <div>
              <div className="eyebrow mb-1.5">Contacts per week</div>
              <div className="flex gap-1.5">
                {[1, 2, 3].map((count) => (
                  <button
                    key={count}
                    onClick={() => setDraft({ ...draft, max_contacts_per_week: count })}
                    className={`num border px-2.5 py-1 text-[0.75rem] ${
                      draft.max_contacts_per_week === count
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
              <div className="eyebrow mb-1.5">Max instalments</div>
              <div className="flex gap-1.5">
                {[1, 2, 3, 4].map((count) => (
                  <button
                    key={count}
                    onClick={() => setDraft({ ...draft, max_installments: count })}
                    className={`num border px-2.5 py-1 text-[0.75rem] ${
                      draft.max_installments === count
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
              <div className="eyebrow mb-1.5">Agent-to-agent</div>
              <button
                onClick={() => setDraft({ ...draft, allow_agent_to_agent: !draft.allow_agent_to_agent })}
                className={`border px-2.5 py-1 text-[0.75rem] ${
                  draft.allow_agent_to_agent
                    ? "border-[var(--color-settled)] text-[var(--color-settled)]"
                    : "border-[var(--color-rule-2)] text-[var(--color-ink-3)]"
                }`}
              >
                {draft.allow_agent_to_agent ? "allowed" : "off"}
              </button>
            </div>
          </div>

          <div className="flex items-center gap-3">
            <button className="btn btn--solid" onClick={() => void save()} disabled={saving}>
              {saving ? "saving…" : "Save mandate"}
            </button>
            <button className="btn btn--ghost" onClick={() => setDraft(mandate)}>
              reset
            </button>
          </div>
        </div>
      )}

      {note && <p className="mono-xs mt-3 rule-t pt-2.5">{note}</p>}
    </section>
  );
}
