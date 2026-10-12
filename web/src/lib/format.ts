export function money(minor: number, currency = "USD"): string {
  const exponent = currency === "JPY" ? 0 : 2;
  const scale = 10 ** exponent;
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor);
  const whole = Math.floor(abs / scale);
  const frac = abs % scale;
  const grouped = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const symbol = currency === "USD" ? "$" : currency === "EUR" ? "€" : currency === "GBP" ? "£" : "";
  if (exponent === 0) return `${sign}${symbol}${grouped}`;
  return `${sign}${symbol}${grouped}.${frac.toString().padStart(2, "0")}`;
}

export function bps(value: number): string {
  const whole = value / 100;
  return Number.isInteger(whole) ? `${whole}%` : `${whole.toFixed(2)}%`;
}

export function shortDate(input: string | null | undefined): string {
  if (!input) return "—";
  const date = new Date(input.length <= 10 ? `${input}T00:00:00Z` : input);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" });
}

export function longDate(input: string | null | undefined): string {
  if (!input) return "—";
  const date = new Date(input.length <= 10 ? `${input}T00:00:00Z` : input);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleDateString("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
}

export function clock(input: string | null | undefined): string {
  if (!input) return "—";
  const date = new Date(input);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" });
}

export function relative(input: string | null | undefined): string {
  if (!input) return "—";
  const date = new Date(input);
  if (Number.isNaN(date.getTime())) return "—";
  const seconds = Math.round((Date.now() - date.getTime()) / 1000);
  if (seconds < 45) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days} d ago`;
  const months = Math.round(days / 30);
  return `${months} mo ago`;
}

export function dayOfMonth(input: string | null | undefined): string {
  if (!input) return "—";
  const date = new Date(input);
  if (Number.isNaN(date.getTime())) return "—";
  const day = date.getUTCDate();
  const suffix = day % 10 === 1 && day !== 11 ? "st" : day % 10 === 2 && day !== 12 ? "nd" : day % 10 === 3 && day !== 13 ? "rd" : "th";
  return `${day}${suffix}`;
}

const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

export function weekdayName(index: number): string {
  return WEEKDAYS[index] ?? "—";
}

export function weekdayList(indexes: number[]): string {
  return indexes.map(weekdayName).join(", ") || "—";
}

/** Terms proposals arrive as minor units; this renders them as one line. */
export function describeTerms(terms: {
  pay_now_minor?: number;
  discount_minor?: number;
  installments?: { amount_minor: number; due: string }[];
  currency?: string;
}): string {
  const currency = terms.currency ?? "USD";
  const parts: string[] = [];
  if (terms.pay_now_minor) parts.push(`${money(terms.pay_now_minor, currency)} now`);
  if (terms.installments?.length) {
    const total = terms.installments.reduce((sum, i) => sum + i.amount_minor, 0);
    parts.push(
      `${money(total, currency)} in ${terms.installments.length} instalment${
        terms.installments.length === 1 ? "" : "s"
      } (${terms.installments.map((i) => shortDate(i.due)).join(", ")})`,
    );
  }
  if (terms.discount_minor) parts.push(`${money(terms.discount_minor, currency)} discount`);
  return parts.join(" · ") || "—";
}
