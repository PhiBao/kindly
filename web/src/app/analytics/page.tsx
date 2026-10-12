"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { AllCommunityModule, ModuleRegistry, themeQuartz, type ColDef } from "ag-grid-community";
import { AgGridReact } from "ag-grid-react";
import { AgCharts } from "ag-charts-react";
import {
  AllCommunityModule as ChartsCommunityModule,
  ModuleRegistry as ChartsModuleRegistry,
  type AgCartesianChartOptions,
  type AgPolarChartOptions,
} from "ag-charts-community";
import { Badge, SectionLabel } from "@/components/primitives";
import type { InvoiceSummary } from "@/lib/api-client";
import { money, shortDate } from "@/lib/format";
import { useBootstrap } from "@/lib/use-bootstrap";

ModuleRegistry.registerModules([AllCommunityModule]);
ChartsModuleRegistry.registerModules([ChartsCommunityModule]);

// ── the model ─────────────────────────────────────────────────────────────
//
// Kindly's forecast is deliberately simple, and the page says so out loud: an
// invoice is expected on its due date plus however late *that client* has
// actually been. Agreed plans use their own dates, because those are
// commitments rather than habits. No black box.

type Band = "reliable" | "watch" | "slow";

interface Reading {
  invoice: InvoiceSummary;
  expected: Date;
  band: Band;
  expectedIn: number;
  week: string;
}

function bandOf(avgDaysLate: number): Band {
  if (avgDaysLate <= 7) return "reliable";
  if (avgDaysLate <= 21) return "watch";
  return "slow";
}

function isoWeekKey(date: Date): string {
  const target = new Date(Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate()));
  const day = target.getUTCDay() || 7;
  target.setUTCDate(target.getUTCDate() + 4 - day);
  const yearStart = new Date(Date.UTC(target.getUTCFullYear(), 0, 1));
  const week = Math.ceil(((target.getTime() - yearStart.getTime()) / 86400000 + 1) / 7);
  return `${target.getUTCFullYear()}–W${String(week).padStart(2, "0")}`;
}

function readBook(invoices: InvoiceSummary[]): Reading[] {
  const today = new Date();
  today.setUTCHours(0, 0, 0, 0);
  return invoices
    .filter((invoice) => invoice.status !== "paid" && invoice.status !== "cancelled")
    .map((invoice) => {
      const due = new Date(`${invoice.due_date}T00:00:00Z`);
      const habit = invoice.behavior?.avg_days_late ?? 0;
      const isPlanInstalment = invoice.number.includes("-P");
      const expected = new Date(due.getTime() + (isPlanInstalment ? 0 : Math.max(0, habit)) * 86400000);
      return {
        invoice,
        expected,
        band: bandOf(habit),
        expectedIn: Math.round((expected.getTime() - today.getTime()) / 86400000),
        week: isoWeekKey(expected),
      };
    })
    .sort((a, b) => a.expected.getTime() - b.expected.getTime());
}

const gridTheme = themeQuartz.withParams({
  backgroundColor: "#fdfaf3",
  foregroundColor: "#16181d",
  borderColor: "#ddd3c0",
  headerBackgroundColor: "#f7f2e8",
  headerTextColor: "#737a8a",
  headerFontWeight: 600,
  fontFamily: "var(--font-inter-tight), system-ui, sans-serif",
  fontSize: "13px",
  cellHorizontalPadding: "10px",
  rowHeight: "38px",
  headerHeight: "34px",
  wrapperBorderRadius: "0px",
  borderRadius: "0px",
  accentColor: "#a32b1e",
});

const INK = "#16181d";
const PAPER_GREY = "#c6b8a0";
const SETTLED = "#1d5b46";
const AMBER = "#a06d12";
const STAMP = "#a32b1e";
const MUTED = "#737a8a";

const axisLabel = { color: MUTED, fontSize: 11 };

export default function AnalyticsPage() {
  const { data, error } = useBootstrap();
  const [metric, setMetric] = useState<"expected" | "outstanding">("expected");

  const readings = useMemo(() => (data ? readBook(data.invoices) : []), [data]);

  const weekSeries = useMemo(() => {
    const buckets = new Map<string, { week: string; reliable: number; watch: number; slow: number }>();
    for (const reading of readings) {
      const bucket =
        buckets.get(reading.week) ?? { week: reading.week, reliable: 0, watch: 0, slow: 0 };
      bucket[reading.band] += reading.invoice.outstanding_minor;
      buckets.set(reading.week, bucket);
    }
    return [...buckets.values()].sort((a, b) => a.week.localeCompare(b.week));
  }, [readings]);

  const clientSeries = useMemo(() => {
    const byClient = new Map<string, { client: string; habit: number; actual: number | null }>();
    for (const invoice of data?.invoices ?? []) {
      const name = invoice.client_name;
      const entry =
        byClient.get(name) ?? { client: name, habit: invoice.behavior?.avg_days_late ?? 0, actual: null };
      entry.habit = Math.max(entry.habit, invoice.behavior?.avg_days_late ?? 0);
      byClient.set(name, entry);
    }
    for (const attribution of data?.attributions ?? []) {
      const entry = byClient.get(attribution.client_name);
      if (entry && attribution.days_to_paid !== null) entry.actual = attribution.days_to_paid;
    }
    return [...byClient.values()];
  }, [data]);

  const outcomeSeries = useMemo(() => {
    const counts = new Map<string, number>();
    for (const attribution of data?.attributions ?? []) {
      counts.set(attribution.kind.replace(/_/g, " "), (counts.get(attribution.kind.replace(/_/g, " ")) ?? 0) + 1);
    }
    return [...counts.entries()].map(([kind, count]) => ({ kind, count }));
  }, [data]);

  const weekOptions: AgCartesianChartOptions = {
    data: weekSeries,
    series: [
      { type: "bar", xKey: "week", yKey: "reliable", yName: "Pays reliably", stacked: true, fill: SETTLED },
      { type: "bar", xKey: "week", yKey: "watch", yName: "Worth watching", stacked: true, fill: AMBER },
      { type: "bar", xKey: "week", yKey: "slow", yName: "Pays slowly", stacked: true, fill: STAMP },
    ],
    axes: {
      x: { type: "category", position: "bottom", label: axisLabel },
      y: {
        type: "number",
        position: "left",
        label: {
          ...axisLabel,
          formatter: ({ value }: { value: number }) => money(value).replace(".00", ""),
        },
      },
    },
    legend: { position: "bottom", item: { label: { color: MUTED, fontSize: 11 } } },
    background: { fill: "transparent" },
  };

  const clientOptions: AgCartesianChartOptions = {
    data: clientSeries,
    series: [
      { type: "bar", xKey: "client", yKey: "habit", yName: "Their habit (days)", fill: PAPER_GREY },
      { type: "bar", xKey: "client", yKey: "actual", yName: "Days to paid (Kindly)", fill: SETTLED },
    ],
    axes: {
      x: { type: "category", position: "bottom", label: axisLabel },
      y: { type: "number", position: "left", label: axisLabel },
    },
    legend: { position: "bottom", item: { label: { color: MUTED, fontSize: 11 } } },
    background: { fill: "transparent" },
  };

  const bandSeries = useMemo(() => {
    const totals = { reliable: 0, watch: 0, slow: 0 };
    for (const reading of readings) totals[reading.band] += reading.invoice.outstanding_minor;
    return [
      { band: "Pays reliably", value: totals.reliable },
      { band: "Worth watching", value: totals.watch },
      { band: "Pays slowly", value: totals.slow },
    ];
  }, [readings]);

  const exposureOptions: AgPolarChartOptions = {
    data: bandSeries,
    series: [
      {
        type: "donut",
        angleKey: "value",
        calloutLabelKey: "band",
        innerRadiusRatio: 0.62,
        fills: [SETTLED, AMBER, STAMP],
        strokes: [SETTLED, AMBER, STAMP],
      },
    ],
    legend: { position: "bottom", item: { label: { color: MUTED, fontSize: 11 } } },
    background: { fill: "transparent" },
  };

  const outcomeOptions: AgCartesianChartOptions = {
    data: outcomeSeries,
    series: [
      { type: "bar", direction: "horizontal", xKey: "kind", yKey: "count", yName: "Invoices", fill: INK },
    ],
    axes: {
      x: { type: "category", position: "left", label: axisLabel },
      y: { type: "number", position: "bottom", label: axisLabel, nice: true },
    },
    background: { fill: "transparent" },
  };

  const columnDefs = useMemo<ColDef<Reading>[]>(
    () => [
      {
        headerName: "Invoice",
        width: 110,
        valueGetter: (params) => params.data?.invoice.number,
        cellRenderer: (params: { value: string; data?: Reading }) => (
          <Link className="num hover:underline" href={`/i/${params.data?.invoice.id}`}>
            {params.value}
          </Link>
        ),
      },
      {
        headerName: "Client",
        valueGetter: (params) => params.data?.invoice.client_name,
        flex: 1,
        minWidth: 140,
      },
      {
        headerName: "Outstanding",
        valueGetter: (params) => params.data?.invoice.outstanding_minor,
        valueFormatter: (params) => money(Number(params.value ?? 0)),
        type: "rightAligned",
        width: 130,
      },
      {
        headerName: "Due",
        valueGetter: (params) => params.data?.invoice.due_date,
        valueFormatter: (params) => shortDate(String(params.value)),
        width: 100,
      },
      {
        headerName: "Late",
        valueGetter: (params) => params.data?.invoice.days_overdue ?? 0,
        valueFormatter: (params) => `${params.value}d`,
        width: 90,
        type: "rightAligned",
      },
      {
        headerName: "Expected in",
        valueGetter: (params) => params.data?.expectedIn ?? 0,
        valueFormatter: (params) => {
          const days = Number(params.value ?? 0);
          return days <= 0 ? "now" : `${days} days`;
        },
        width: 120,
        type: "rightAligned",
      },
      {
        headerName: "Expected on",
        valueGetter: (params) => params.data?.expected.toISOString().slice(0, 10),
        valueFormatter: (params) => shortDate(String(params.value)),
        width: 120,
      },
      {
        headerName: "Reads as",
        valueGetter: (params) => params.data?.band,
        width: 110,
        cellRenderer: (params: { value: Band }) => (
          <Badge tone={params.value === "reliable" ? "settled" : params.value === "slow" ? "stamp" : "amber"}>
            {params.value}
          </Badge>
        ),
      },
      {
        headerName: "Where it stands",
        valueGetter: (params) => params.data?.invoice.negotiation.stage_label,
        flex: 1,
        minWidth: 150,
      },
    ],
    [],
  );

  return (
    <div className="mx-auto w-full max-w-[1240px] px-5 pb-24 sm:px-8">
      <header className="rule-heavy-b flex flex-wrap items-end justify-between gap-4 pb-4 pt-7">
        <div>
          <Link href="/" className="eyebrow hover:text-[var(--color-ink)]">
            ← the desk
          </Link>
          <h1 className="display mt-2 text-[2rem] leading-none">When will I actually get paid?</h1>
          <p className="mt-1.5 max-w-[74ch] text-[0.875rem] text-[var(--color-ink-2)]">
            The register and the money view, built on AG Grid and AG Charts. Every expected date is due date
            plus what that client has actually done before — and agreed payment plans use their own dates,
            because those are commitments rather than habits.
          </p>
        </div>
        {data && (
          <div className="flex items-center gap-2.5">
            <Badge tone="neutral">ag grid community</Badge>
            <Badge tone="neutral">{readings.length} open</Badge>
          </div>
        )}
      </header>

      {error && <p className="mono-xs mt-6 text-[var(--color-stamp)]">{error}</p>}

      {data && (
        <>
          <section className="mt-8">
            <SectionLabel
              aside={
                <div className="flex gap-1.5">
                  {(["expected", "outstanding"] as const).map((option) => (
                    <button
                      key={option}
                      onClick={() => setMetric(option)}
                      className={`btn btn--sm ${metric === option ? "btn--solid" : ""}`}
                    >
                      {option === "expected" ? "by expected week" : "by risk"}
                    </button>
                  ))}
                </div>
              }
            >
              Expected inflows
            </SectionLabel>
            <div className="paper-edge p-3">
              {metric === "expected" ? (
                <AgCharts style={{ height: 300 }} options={weekOptions} />
              ) : (
                <AgCharts style={{ height: 300 }} options={exposureOptions} />
              )}
            </div>
            <p className="mono-xs mt-2">
              {metric === "expected"
                ? "Each bar is a week the money is expected to land; the colour is how that client behaves."
                : "How much of what you are owed sits behind clients who pay reliably, slowly, or somewhere in between."}
            </p>
          </section>

          <section className="mt-10">
            <SectionLabel aside={<span className="mono-xs">sortable · filterable</span>}>
              The register, read forward
            </SectionLabel>
            <div
              style={{ height: Math.min(560, 78 + readings.length * 38) }}
              className="paper-edge"
            >
              <AgGridReact<Reading>
                theme={gridTheme}
                rowData={readings}
                columnDefs={columnDefs}
                defaultColDef={{ sortable: true, resizable: false, filter: false }}
                ensureDomOrder
              />
            </div>
          </section>

          <div className="mt-10 grid gap-10 lg:grid-cols-2">
            <section>
              <SectionLabel aside={<span className="mono-xs">habit vs outcome</span>}>
                How late each client really pays
              </SectionLabel>
              <div className="paper-edge p-3">
                <AgCharts style={{ height: 280 }} options={clientOptions} />
              </div>
            </section>

            <section>
              <SectionLabel aside={<span className="mono-xs">attributed cause</span>}>
                What actually got you paid
              </SectionLabel>
              {outcomeSeries.length === 0 ? (
                <p className="text-[0.8125rem] text-[var(--color-ink-3)]">
                  Nothing settled yet — the first settled invoice appears here with its cause.
                </p>
              ) : (
                <div className="paper-edge p-3">
                  <AgCharts style={{ height: 280 }} options={outcomeOptions} />
                </div>
              )}
            </section>
          </div>

          <section className="mt-10">
            <SectionLabel>How the reading works</SectionLabel>
            <div className="grid gap-6 sm:grid-cols-3">
              <div className="rule-t pt-2.5">
                <div className="eyebrow">Expected date</div>
                <p className="num mt-1.5 text-[0.8125rem]">due + this client&rsquo;s average lateness</p>
                <p className="mt-1.5 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
                  Measured from settled invoices only. A client with no history reads as unknown rather than
                  guessed.
                </p>
              </div>
              <div className="rule-t pt-2.5">
                <div className="eyebrow">Plan dates</div>
                <p className="num mt-1.5 text-[0.8125rem]">instalments use their agreed dates</p>
                <p className="mt-1.5 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
                  Once terms are agreed the dates are commitments — the habit model steps aside.
                </p>
              </div>
              <div className="rule-t pt-2.5">
                <div className="eyebrow">Correction</div>
                <p className="num mt-1.5 text-[0.8125rem]">every outcome updates the habit</p>
                <p className="mt-1.5 text-[0.75rem] leading-snug text-[var(--color-ink-3)]">
                  Each payment recorded re-derives that client&rsquo;s average, so next month&rsquo;s reading is
                  sharper than this one.
                </p>
              </div>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
