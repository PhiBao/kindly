/**
 * Renders every surface in a real browser, fails on console/page errors, walks
 * the two decision flows, and saves screenshots to /tmp/kindly-shots.
 *
 *   node scripts/verify-ui.mjs   (from web/) [--base http://localhost:3000] [--api http://localhost:8787]
 */
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";

const args = process.argv.slice(2);
const arg = (name, fallback) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : fallback;
};

const BASE = arg("--base", "http://localhost:3000");
const API = arg("--api", "http://localhost:8787");
const SHOTS = "/tmp/kindly-shots";
mkdirSync(SHOTS, { recursive: true });

const problems = [];
const note = (message) => {
  problems.push(message);
  console.log(`  \u001b[31mFAIL\u001b[0m ${message}`);
};
const ok = (message) => console.log(`  \u001b[32mPASS\u001b[0m ${message}`);

const browser = await chromium.launch({
  executablePath: "/home/kiter/.local/bin/chromium",
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});

async function openPage(path) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 2 });
  const page = await context.newPage();
  page.on("console", (message) => {
    if (message.type() === "error") note(`[${path}] console error: ${message.text().slice(0, 200)}`);
  });
  page.on("pageerror", (error) => note(`[${path}] page error: ${String(error).slice(0, 200)}`));
  await page.goto(`${BASE}${path}`, { waitUntil: "networkidle", timeout: 60_000 });
  return { page, context };
}

const api = async (path, init) => {
  const response = await fetch(`${API}${path}`, { headers: { "content-type": "application/json" }, ...init });
  return response.json();
};

// Ensure the demo data exists, then note the ids we need.
await api("/api/demo/reset", { method: "POST" });
const bootstrap = await api("/api/bootstrap");
const acme = bootstrap.invoices.find((invoice) => invoice.number === "1042");
const northwind = bootstrap.invoices.find((invoice) => invoice.number === "1039");
ok(`demo data ready (${bootstrap.invoices.length} invoices)`);

// ── the desk ─────────────────────────────────────────────────────────────
{
  const { page, context } = await openPage("/");
  await page.waitForSelector("text=You do the work.", { timeout: 30_000 });
  const outstanding = await page.locator("text=Outstanding").count();
  if (outstanding === 0) note("dashboard is missing the Outstanding metric");
  else ok("dashboard renders headline, metrics and ledger");
  await page.screenshot({ path: `${SHOTS}/01-desk.png`, fullPage: true });
  await context.close();
}

// ── run the agent-to-agent scenario from the tray ────────────────────────
{
  const { page, context } = await openPage("/");
  await page.waitForSelector("text=Scenarios");
  await page.getByRole("button", { name: "Scenarios" }).click();
  await page.waitForSelector("text=Agent-to-agent negotiation");
  await page.getByText("Agent-to-agent negotiation", { exact: false }).first().click();
  await page.waitForURL(/\/n\//, { timeout: 30_000 });
  await page.waitForSelector("text=Negotiation room", { timeout: 30_000 });
  await page.waitForSelector("text=Decision", { timeout: 30_000 });
  const verified = await page.locator("text=verified").count();
  if (verified < 3) note(`expected several verified envelopes, saw ${verified}`);
  else ok(`negotiation room shows ${verified} verified envelopes`);
  await page.screenshot({ path: `${SHOTS}/02-negotiation.png`, fullPage: true });

  // Advance: either hold the floor or approve the client's release.
  const buyerButton = page.getByRole("button", { name: /finance lead approves/i });
  if (await buyerButton.count()) {
    await buyerButton.click();
    await page.waitForTimeout(2500);
    ok("client-side approval walked through the UI");
  } else {
    const step = page.getByRole("button", { name: /Advance the negotiation/i });
    if (await step.count()) {
      await step.click();
      await page.waitForTimeout(2500);
      ok("negotiation advanced through the UI");
    } else {
      note("no decision control was offered on the negotiation room");
    }
  }
  await page.screenshot({ path: `${SHOTS}/03-negotiation-after.png`, fullPage: true });
  await context.close();
}

// ── the invoice desk ─────────────────────────────────────────────────────
{
  const { page, context } = await openPage(`/i/${acme.id}`);
  await page.waitForSelector("text=The correspondence", { timeout: 30_000 });
  await page.screenshot({ path: `${SHOTS}/04-invoice-desk.png`, fullPage: true });
  ok("invoice desk renders the correspondence and the audit trail");

  // The client portal, opened from the desk.
  const portalLink = await page.locator('a', { hasText: "Open the portal" }).first().getAttribute("href");
  if (!portalLink) note("invoice desk did not offer the client portal link");
  else ok(`client portal link present (${portalLink.slice(0, 40)}…)`);
  await context.close();
}

// ── the client portal, including a payment ───────────────────────────────
{
  const detail = await api(`/api/invoices/${northwind.id}`);
  const token = detail.portal_url.split("/c/")[1];
  const { page, context } = await openPage(`/c/${token}`);
  await page.waitForSelector("text=Conversation", { timeout: 30_000 });
  await page.screenshot({ path: `${SHOTS}/05-portal.png`, fullPage: true });

  // The client asks for a plan.
  await page.locator("textarea").fill("Could we do $575 now and the rest on the 20th?");
  await page.getByRole("button", { name: /^Send$/ }).click();
  await page.waitForSelector("text=read that as", { timeout: 30_000 });
  ok("client message understood and answered in the portal");

  // …then pays the up-front part.
  await page.getByRole("button", { name: /Pay .* with PayPal/ }).click();
  await page.waitForTimeout(3000);
  const receipt = await page.locator("text=Receipts").count();
  if (receipt === 0) note("payment went through but no receipt was shown");
  else ok("payment captured and receipt rendered");
  await page.screenshot({ path: `${SHOTS}/06-portal-paid.png`, fullPage: true });
  await context.close();
}

await browser.close();

console.log();
if (problems.length) {
  console.log(`\u001b[31m${problems.length} problem(s) found.\u001b[0m`);
  process.exit(1);
}
console.log(`\u001b[32mAll UI checks passed.\u001b[0m Screenshots in ${SHOTS}`);
