import type React from "react";

/** The mark: a stamped K. Also the favicon (see app/icon.svg). */
export function Wordmark({ size = 22, withTagline = false }: { size?: number; withTagline?: boolean }) {
  return (
    <span className="inline-flex items-baseline gap-2.5">
      <span className="display leading-none" style={{ fontSize: size * 1.6, letterSpacing: "-0.03em" }}>
        Kindly
      </span>
      {withTagline && (
        <span className="eyebrow hidden sm:inline" style={{ letterSpacing: "0.18em" }}>
          the asking, handled
        </span>
      )}
    </span>
  );
}

export function StampK({ size = 26 }: { size?: number }) {
  return (
    <span
      aria-hidden
      className="stamp shrink-0"
      style={{ fontSize: size * 0.42, padding: `${size * 0.1}px ${size * 0.16}px`, transform: "rotate(-6deg)" }}
    >
      K
    </span>
  );
}

type StampTone = "stamp" | "settled" | "quiet";

export function Stamp({
  label,
  tone = "stamp",
  live = false,
  title,
}: {
  label: string;
  tone?: StampTone;
  live?: boolean;
  title?: string;
}) {
  const toneClass = tone === "settled" ? "stamp--settled" : tone === "quiet" ? "stamp--quiet" : "";
  return (
    <span className={`stamp ${toneClass} ${live ? "stamp--live" : ""}`} title={title}>
      {label}
    </span>
  );
}

export function SectionLabel({ children, aside }: { children: React.ReactNode; aside?: React.ReactNode }) {
  return (
    <div className="rule-heavy-b mb-3 flex items-baseline justify-between gap-4 pb-1.5">
      <div className="section-label flex-1">
        <span className="eyebrow">{children}</span>
      </div>
      {aside}
    </div>
  );
}

export function Rule({ double = false }: { double?: boolean }) {
  return <div className={double ? "rule-heavy-t" : "rule-t"} />;
}

export function Metric({
  label,
  value,
  hint,
  tone = "ink",
}: {
  label: string;
  value: string;
  hint?: string;
  tone?: "ink" | "stamp" | "settled";
}) {
  const color =
    tone === "stamp" ? "var(--color-stamp)" : tone === "settled" ? "var(--color-settled)" : "var(--color-ink)";
  return (
    <div className="min-w-0">
      <div className="eyebrow">{label}</div>
      <div className="num mt-1 text-[1.35rem] leading-none" style={{ color }}>
        {value}
      </div>
      {hint && <div className="mono-xs mt-1">{hint}</div>}
    </div>
  );
}

export function Badge({
  children,
  tone = "neutral",
}: {
  children: React.ReactNode;
  tone?: "neutral" | "stamp" | "settled" | "amber";
}) {
  const tones: Record<string, { color: string }> = {
    neutral: { color: "var(--color-ink-3)" },
    stamp: { color: "var(--color-stamp)" },
    settled: { color: "var(--color-settled)" },
    amber: { color: "var(--color-amber)" },
  };
  return (
    <span
      className="mono-xs inline-flex shrink-0 items-center border px-1.5 py-0.5 uppercase"
      style={{ ...tones[tone], borderColor: "currentColor", opacity: 0.95, letterSpacing: "0.08em" }}
    >
      {children}
    </span>
  );
}

export function StatusLine({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: React.ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="rule-b flex items-baseline justify-between gap-4 py-[0.4rem]">
      <span className="text-[0.8125rem] text-[var(--color-ink-3)]">{label}</span>
      <span className={mono ? "num min-w-0 truncate text-right text-[0.8125rem]" : "text-right text-[0.8125rem] font-medium"}>
        {value}
      </span>
    </div>
  );
}

export function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[0.8125rem] leading-relaxed text-[var(--color-ink-3)]">{children}</p>
  );
}
