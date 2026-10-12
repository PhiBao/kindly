"use client";

import Link from "next/link";
import { useState } from "react";
import { Badge, StampK, Wordmark } from "@/components/primitives";
import { api, type Bootstrap, type Scenario } from "@/lib/api-client";

export function DeskShell({
  data,
  onRefresh,
  children,
  actions,
}: {
  data: Bootstrap | null;
  onRefresh: () => void;
  children: React.ReactNode;
  actions?: React.ReactNode;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [trayOpen, setTrayOpen] = useState(false);
  const [trayNote, setTrayNote] = useState<string | null>(null);
  const mode = data?.owner.paypal_mode_raw ?? "sim";

  const runScenario = async (scenario: Scenario) => {
    setBusy(scenario.name);
    setTrayNote(null);
    try {
      const result = (await api.scenario(scenario.name)) as Record<string, unknown>;
      const session = (result.session ?? null) as { id?: string } | null;
      if (session?.id) {
        window.location.href = `/n/${session.id}`;
        return;
      }
      if (scenario.name === "injection") {
        setTrayNote(
          `Read as “${result.reading}”. Money unchanged: ${result.outstanding_before} → ${result.outstanding_after}.`,
        );
      } else if (scenario.name === "quiet_hours") {
        const night = (result.at_0300 as { explanation?: string } | undefined)?.explanation;
        setTrayNote(night ?? null);
      } else {
        setTrayNote("Done — the ledger has been updated.");
      }
      onRefresh();
    } catch (cause) {
      setTrayNote(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  };

  const runAgent = async () => {
    setBusy("tick");
    try {
      await api.tickAll();
      onRefresh();
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="mx-auto w-full max-w-[1240px] px-5 pb-24 sm:px-8">
      <header className="rule-heavy-b flex flex-wrap items-end justify-between gap-4 pb-4 pt-7">
        <div className="flex items-end gap-3.5">
          <Link href="/" className="flex items-end gap-3">
            <StampK size={30} />
            <Wordmark size={23} />
          </Link>
          {data && (
            <span className="eyebrow pb-1 hidden md:inline">
              {data.owner.business} · {data.clients.length} clients · {data.invoices.length} invoices
            </span>
          )}
        </div>

        <div className="flex flex-wrap items-center gap-2.5">
          {actions}
          <Link className="btn" href="/analytics">
            The money view
          </Link>
          <button className="btn" onClick={runAgent} disabled={busy !== null}>
            {busy === "tick" ? "Working…" : "Run the agent"}
          </button>
          <button
            className={`btn ${trayOpen ? "btn--solid" : ""}`}
            onClick={() => setTrayOpen((open) => !open)}
          >
            Scenarios
          </button>
          {data && (
            <div className="flex items-center gap-2 rule-l pl-2.5">
              <Badge tone={mode === "live" ? "settled" : "amber"}>
                {mode === "live" ? "PayPal sandbox" : "simulated PayPal"}
              </Badge>
              <Badge tone={data.environment.model_enabled ? "settled" : "neutral"}>
                {data.environment.model_enabled ? "model live" : "template voice"}
              </Badge>
            </div>
          )}
        </div>
      </header>

      {trayOpen && data && (
        <section className="paper-edge mt-4 p-4 sm:p-5">
          <div className="flex items-baseline justify-between gap-4">
            <div>
              <div className="eyebrow">Demo scenarios</div>
              <p className="mt-1 text-[0.8125rem] text-[var(--color-ink-2)]">
                Each one drives the real pipeline — no scripted responses. Open a scenario and watch the ledger.
              </p>
            </div>
            <button className="btn btn--ghost btn--sm" onClick={() => setTrayOpen(false)}>
              close
            </button>
          </div>
          <div className="mt-4 grid gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
            {data.scenarios.map((scenario) => (
              <button
                key={scenario.name}
                onClick={() => void runScenario(scenario)}
                disabled={busy !== null}
                className="rule-t pt-2.5 text-left transition-colors hover:bg-[color-mix(in_srgb,var(--color-ink)_3%,transparent)] disabled:opacity-50"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[0.8125rem] font-medium">{scenario.title}</span>
                  <span className="mono-xs">#{scenario.invoice}</span>
                </div>
                <p className="mt-1 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
                  {scenario.description}
                </p>
                {busy === scenario.name && <span className="mono-xs mt-1 block">running…</span>}
              </button>
            ))}
          </div>
          {trayNote && (
            <p className="mono-xs mt-3 rule-t pt-2.5 whitespace-pre-wrap text-[var(--color-ink-2)]">{trayNote}</p>
          )}
        </section>
      )}

      <main className="mt-8">{children}</main>

      <footer className="rule-heavy-t mt-16 pt-4 flex flex-wrap items-baseline justify-between gap-3">
        <span className="eyebrow">Kindly · the asking, handled</span>
        <span className="mono-xs">
          {data ? `${data.stats.agent_actions} agent actions · ${data.stats.paypal_events} PayPal events` : ""}
        </span>
      </footer>
    </div>
  );
}
