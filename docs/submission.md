# Devpost submission — answers ready to paste

Everything the submission form asks for, with the reasoning kept out of the paste-ready
blocks. Replace `‹placeholders›` before submitting.

---

## Submitter

| Field | Value |
| --- | --- |
| Submitter Type | `Individual` (or `Team` / `Organization` as appropriate) |
| Submitter Country of Residence | `‹your country›` |
| If you or any team members reside in Canada | `N/A` (or the province) |
| Organization Name | `N/A` |
| Is your project new, or did it exist before Oct 1, 2026? | **New** — the first commit is dated `‹date of first commit›` |
| If it existed before, explain… | `N/A — new project.` |

---

## Explain how your project meets the main technology requirement

> **PayPal tools.** Kindly integrates the PayPal developer platform as the only rail it moves
> money on:
>
> - **Invoicing v2** — create draft invoice, send, send reminder (respecting PayPal's
>   two-reminders-per-day cap), record payment (full and partial), cancel, show invoice, list
>   invoices for reconciliation, generate QR code. Every write carries a `PayPal-Request-Id`
>   idempotency key and retries with backoff on 429/5xx.
> - **Invoicing webhooks** — `INVOICING.INVOICE.CREATED / SENT / UPDATED / PAID / CANCELLED`,
>   verified through PayPal's `verify-webhook-signature` endpoint, idempotent by event id, raw
>   deliveries stored.
> - **Checkout Orders v2 + captures** — the client portal's "Pay now" and "Pay part" buttons
>   create an order and capture it; the invoice id travels in `custom_id`, so a capture can
>   settle correctly even if the HTTP response to the app is lost.
> - **`PAYMENT.CAPTURE.COMPLETED` webhooks** — the authoritative record that money moved,
>   applied against the right invoice and its parent plan.
> - **Agent tools parity** — the agent's tool layer mirrors PayPal's published agent tools
>   (`create_invoice`, `send_invoice`, `send_reminder`, `record_payment`), and the seller agent
>   signs the mandate it acts under with Ed25519, in the spirit of AP2 mandates.
>
> All of it runs against the **PayPal sandbox** in `KINDLY_PAYPAL_MODE=live`; the default mode
> is an in-process simulator that implements the same interface, signs the same webhook
> deliveries and enforces the same caps, so the project can be run and judged with no
> credentials at all.
>
> **AI tools.** Kindly uses an OpenAI-compatible language model (`gpt-4o-mini` by default via
> `KINDLY_LLM_API_KEY`; any OpenAI-compatible endpoint works) for four bounded jobs: reading
> free-form client replies into closed intents, composing messages in the owner's voice,
> proposing payment-plan parameters, and choosing the next move from a closed list. The model
> never authorizes anything: every proposal is judged by a deterministic policy engine against
> the user's mandate, and the verdict — allowed, needs approval, or blocked — is written to an
> append-only ledger. With no model configured the product still runs, falling back to
> deterministic templates and heuristic understanding.

---

## Which additional sponsor tool(s) did you use?

> **AG Grid** — the *money view* (`/analytics`) is built on AG Grid (the receivables register,
> sortable, with money formatting and risk renderings) and AG Charts (expected inflows by
> week, exposure by client behaviour, habit-versus-outcome and attributed-cause charts). The
> expected-payment model is stated in the UI rather than hidden.
>
> No other sponsor tool is claimed in this submission — AG Grid is the one used in the product
> itself.

### If you used AG Grid, what was your experience building with AG Grid/AG Charts?

> Used AG Grid community + AG Charts as the register and charts for a receivables book:
> sorting a ledger of invoices read forward to expected payment dates, with money in tabular
> figures and the client's payment behaviour rendered as a badge. What worked well: the
> Theming API made it straightforward to adopt our own design tokens (paper background, ink
> text, hairline borders, mono numerals) so the grid does not look like a bolted-on widget;
> column definitions with `valueGetter`/`valueFormatter` kept the money formatting in one
> place. Friction worth noting: AG Charts v14 requires module registration
> (`ModuleRegistry.registerModules`) before render, and passing `axes` as an array type-checks
> against the polar axes union — using the record form (`axes: { x: …, y: … }`) and typing the
> options as `AgCartesianChartOptions` resolved it.

## Provide a URL to your public code repository

> `‹https://github.com/…/kindly›` — MIT licensed, with the licence visible in the About
> section. The README covers what it is, how to run it in simulation mode with no credentials,
> how to point it at a PayPal sandbox, how to run the unit tests, the end-to-end smoke test and
> the browser check, and an explicit table of what is real versus simulated.

## Provide a URL to a working demo

> `‹https://kindly-…onrender.com›` — hosted on Render in simulation mode. The dashboard is
> open (this deployment moves no real money, and every screen says so).

## Testing instructions or credentials

> Nothing is required to start: the hosted demo is open, and the repository runs with
> `./scripts/dev-server.sh` (no PayPal account, no API key).
>
> **The fastest path to the interesting part (90 seconds):**
>
> 1. On the desk, press **Scenarios → Agent-to-agent negotiation**. You land in the negotiation
>    room: Kindly's offer, Acme's counter, the automatic refusal (their counter broke the
>    owner's 70% floor), Kindly holding the line, and their agent escalating to a human.
> 2. Press **Their finance lead approves** — Kindly issues the deferred part as PayPal invoices.
> 3. Open the **invoice desk** for 1042, copy the **client link**, and open it. You are the
>    client now: type "could we do $575 now and the rest on the 20th?" and press Send, then
>    pay part of the invoice.
> 4. Back on the desk, read the agent's trail and the proof panel; press **Scenarios → A client
>    tries to instruct the agent** to see the defence.
>
> For a full verification run: `python3 scripts/smoke-demo.py` (34 checks) and
> `cd web && node scripts/verify-ui.mjs`.

---

## How would you rate your overall experience integrating PayPal?

> **8 — good, with sharp edges worth knowing.**

### Please share why or provide additional feedback

> The Invoicing v2 API was the right primitive for this product — invoices, reminders,
> partial payments and webhooks are exactly what a collections agent needs, and the sandbox is
> faithful enough that we could build against it confidently. Documentation quality is high.
> Things that cost us time: (1) webhook verification requires a publicly reachable URL, which
> makes local development awkward — a sandbox-only "accept signed test deliveries" path or a
> first-class CLI tunnelling story would help a lot; (2) growing a partial payment into a
> settled invoice needs several round trips (show invoice → record payment → reconcile), where
> a single "apply payment" response that returns the new balance would be cleaner; (3) the
> two-reminders-per-day rule is documented but easy to trip; it failed loudly in our simulator
> (by design) which is how we found it; (4) the agent toolkit is TypeScript/Python only, so a
> Rust service has to hand-roll the OAuth/tool layer — publishing the tool schemas
> independently of the SDKs would let any language implement it faithfully (we mirrored the
> published tool names for exactly that reason).

---

## Which AI tools have you leveraged while working on this project?

> OpenAI-compatible chat completions for the product's own drafting and understanding (with a
> deterministic template fallback), a coding agent (OpenCode) for implementation, review and
> verification, and Playwright for browser-level checks of every surface.

## Describe the level of learning you/your team derived from the project

> **Significant.**

## Did you gain AI value that you can use in your career?

> **Yes.**

---

## Three-minute video

> `‹YouTube URL — unlisted is fine, must be publicly playable›`
> Script and shot list: `docs/demo-script.md`.
