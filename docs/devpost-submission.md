# Devpost submission — final copy

Status: **Submitted** to the PayPal AI Hackathon on 2 November 2026.
Devpost: <https://devpost.com/software/kindly-cbf7gt>

Project: **Kindly**
Video: <https://youtu.be/Ib4FgWIXMnk>
Repo: <https://github.com/PhiBao/kindly>
Demo: <https://kindly-desk-web.vercel.app>
Simulation: <https://kindly-desk-sim.vercel.app>

---

## Tagline (200 chars)

An agent that chases, negotiates and settles unpaid invoices through PayPal, inside limits you write once.

---

## Description

**Kindly chases, negotiates and settles unpaid invoices through PayPal. You set the limits once, in plain language, and the agent works inside them.**

I built Kindly for freelancers and small studios. The work is usually fine. The follow-up is what hurts. You finish a project, send the invoice, and then have to become the person who asks for money.

### The problem

Late payment is not an edge case.

- 59% of small businesses have invoices more than 30 days overdue, and the average business waiting on payment is owed $17,700 (Intuit QuickBooks, 2026 Small Business Late Payments Report).
- Small businesses wait 29.3 days to get paid, roughly 9 days late (Xero Small Business Insights, Q2 2026).
- 85% of freelancers are affected by late payment, and 42% have missed personal bills because a client paid late (Plutio / Freelancer Playbook, 2026).
- FreshBooks and Stripe's 2026 State of Financial Flow names the pain directly: "uncomfortable client conversations." Freelancers build reminder automations so the ask comes from a system, not from them.

Reminder tools send the email and stop. The freelancer still has to decide what to concede, say it, and chase whatever was agreed. Kindly does that part. And because the money moves through PayPal, it can also show which action caused the payment.

### What it does

A freelancer opens the desk and sees their receivables. Kindly reads the situation (how late, how this client usually pays, what they have already said) and proposes a move: a nudge, a firmer ask, a payment plan, an escalation, or closing the invoice as uncollectible. Every proposal goes through the mandate before anything is sent.

The client gets a signed, expiring link. No account. They can read the letters, pay with PayPal, or ask for different terms in their own words. Kindly reads the intent, checks it against the mandate, and answers in the studio's voice.

If the client's side has an accounts-payable agent, Kindly negotiates with it directly: signed terms, a counter, an automatic refusal when the counter breaks the floor, and a human approval on each side before money moves.

### How the limits work

The mandate is a small set of rules the owner writes once: contacts per week, quiet hours, the minimum up-front share, the discount ceiling, the number of instalments, how long a plan can run, and whether the agent may negotiate with another agent.

A deterministic policy engine evaluates every proposal against those rules and returns allow, needs approval, or blocked. The language model never authorizes anything. All three verdicts go into an append-only ledger with the reason attached, so a refusal is as visible as a success. In the demo you can watch it refuse Acme's counter because 50% up front is below the owner's 70% floor.

### PayPal, end to end

- **Invoicing v2** creates real PayPal invoice objects, sends them, records payments, cancels, lists, and generates QR codes. If an app is not granted the Invoicing feature, Kindly falls back to a Checkout payment link and records why.
- **Checkout Orders v2** powers the portal's pay-now and pay-part buttons. The invoice id travels in `custom_id`, so a capture still settles the right invoice if the HTTP response is lost.
- **Webhooks** for `INVOICING.INVOICE.*`, `CHECKOUT.ORDER.APPROVED` and `PAYMENT.CAPTURE.COMPLETED` are verified through PayPal's `verify-webhook-signature` endpoint against the exact delivered bytes, deduplicated by event id, and stored.
- Every write carries a `PayPal-Request-Id` idempotency key and retries with backoff on 429/5xx.
- The hosted demo runs against the sandbox. It has created invoice objects, opened checkout, captured a test-card payment and received the signed capture event. The testing instructions below let you do the same.

### Where AI is used

Kindly separates authority, judgment, and language.

- **TypeSafe System One (Jev)** answers typed questions: a Choice over a closed intent set, a Noul asking whether the message addresses the system rather than the business, and a Choice over the candidate moves. Every answer carries a probability, and a low-confidence answer falls back to a deterministic reader instead of acting on a coin flip.
- I calibrated the instruction-attempt threshold on eleven real replies. Attempts scored 0.81-0.98; ordinary correspondence scored 0.02-0.14. The 0.60 cutoff sits inside that gap.
- Prose is optional. Without a model, Kindly writes from templates. With an OpenAI-compatible endpoint, the letters are generated, but the plan parameters are still clamped in code against the mandate.

### What I verified

- 101 Rust tests, 39 end-to-end checks against the simulation, 11 browser checks against the deployed pair.
- `scripts/verify-live.py` reads every mirrored invoice back from PayPal with the app's own credentials, confirms every recorded delivery was signature-verified, and opens a Checkout order without approving it.
- The video shows a test-card payment on the live sandbox, the capture, the signed webhook, and the attribution ("paid on their own, 10 days ahead of their usual").

### Links

- Live demo: <https://kindly-desk-web.vercel.app>
- Simulation: <https://kindly-desk-sim.vercel.app>
- Code: <https://github.com/PhiBao/kindly> (MIT)
- Video: <https://youtu.be/Ib4FgWIXMnk>

---

## 1. Submitter type

Individual.

## 2. Country of residence

Vietnam.

## 3. Canada province

N/A

## 4. Organization name

N/A

## 5. New or existing

New.

## 6. If existing, what changed

N/A. The project is new; the first commit is dated 12 October 2026.

## 7. How the project meets the main technology requirement

PayPal is the only rail Kindly moves money on, and the AI makes the judgment calls.

**PayPal.** Invoicing v2 creates and sends real invoice objects, records partial payments, cancels, lists, and generates QR codes. Checkout Orders v2 and captures power the client portal's pay-now and pay-part flows. Webhooks for `INVOICING.INVOICE.CREATED`, `SENT`, `UPDATED`, `PAID` and `CANCELLED`, plus `CHECKOUT.ORDER.APPROVED` and `PAYMENT.CAPTURE.COMPLETED`, are verified with `verify-webhook-signature` against the exact delivered bytes and deduplicated by event id. OAuth client credentials use a cached token, retries use backoff, and every write carries a `PayPal-Request-Id`.

**AI.** TypeSafe System One (Jev) answers three typed questions per client reply and per planning step: a Choice over a closed intent set, a Noul for whether the message addresses the system rather than the business, and a Choice over the candidate moves. The probabilities are stored in the ledger, and a confidence below the calibrated threshold falls back to the deterministic reader. An OpenAI-compatible endpoint can write the prose when a key is configured; without it the letters come from templates. The policy engine is deliberately not a model. It is a pure function over the mandate, so no generation can authorize a payment.

## 8. Additional sponsor tools

AG Grid.

## 9. AG Grid experience

I used AG Grid Community and AG Charts for the money view: a sortable receivables register read forward to expected payment dates, with money in tabular figures and each client's payment behaviour rendered as a badge.

What worked well: the Theming API let me adopt the product's own design tokens (paper background, ink text, hairline rules, mono numerals) so the grid does not look like a bolted-on widget. `valueGetter` and `valueFormatter` kept money formatting in one place.

Friction worth noting: AG Charts v14 needs module registration (`ModuleRegistry.registerModules`) before anything renders, and passing `axes` as an array type-checks against the polar axes union. Using the record form (`axes: { x, y }`) and typing the options as `AgCartesianChartOptions` fixed it.

## 10. Code repository

<https://github.com/PhiBao/kindly>

Public, MIT licensed, with the licence visible in the About section. The README covers the product, the architecture, how to run it in simulation with no credentials, how to point it at a PayPal sandbox, the tests, and an explicit table of what is real and what is simulated.

## 11. Working demo

<https://kindly-desk-web.vercel.app>

Next.js on Vercel, Rust API on Fly, SQLite on a Fly volume, running against the PayPal sandbox. The demo is open and reseeds to a four-invoice story. A simulation deployment runs the same product with an in-process PayPal at <https://kindly-desk-sim.vercel.app>.

## 12. Testing instructions

The demo is open; no login is required.

1. On the desk, press **Scenarios → Agent-to-agent negotiation**. You land in the negotiation room: Kindly's offer, Acme's counter, the automatic refusal because the counter breaks the 70% floor, and their agent escalating to a human.
2. Press **Their finance lead approves**. Kindly issues the deferred part as two real PayPal invoices.
3. Open the invoice desk for 1042, copy the client link, and open it. You are the client now. Type "could we split this over two months?" and press Send; Kindly reads it as a plan request and answers in the studio's voice.
4. Press **Pay with PayPal** to see the live sandbox checkout. Use card `4111 1111 1111 1111`, any future expiry, CVC `123`, and any US billing address. At PayPal choose **Pay with debit or credit card**, then **Pay now as guest**. You come back to a settled invoice, and the capture appears in the desk's PayPal deliveries.
5. If you would rather not enter card details, <https://kindly-desk-sim.vercel.app> is the same product against an in-process PayPal, where payment is one click.

## 13. PayPal experience rating

8.

## 14. Why that rating

The Invoicing v2 API was the right primitive for this product. Invoices, reminders, partial payments and webhooks are exactly what a collections agent needs, and the sandbox is faithful enough to build against with confidence. The documentation is good.

Things that cost me time:

1. The Invoicing **feature** is a separate grant from valid credentials. With correct keys, OAuth, Checkout and webhooks all worked while `POST /v2/invoicing/invoices` returned `403 NOT_AUTHORIZED`. The token's scope list is the only clue, and the error does not name the missing feature.
2. Webhook verification requires a publicly reachable URL, which makes local development awkward. A sandbox-only path for signed test deliveries, or a first-class tunnelling story, would help.
3. Growing a partial payment into a settled invoice takes several round trips (show invoice, record payment, reconcile). A single apply-payment response that returns the new balance would be cleaner.
4. The two-reminders-per-day cap is documented but easy to trip. It failed loudly in my simulator, which is how I found it.
5. The agent toolkit is TypeScript and Python only, so a Rust service has to hand-roll the OAuth and tool layer. Publishing the tool schemas independently of the SDKs would let any language implement it faithfully.

## 15. Contact for follow-up feedback

Yes.

## 16. AI tools used while building

TypeSafe System One (Jev) for the product's typed judgments and the calibration harness; an OpenAI-compatible chat endpoint for optional prose, with deterministic templates by default; OpenCode as the coding agent for implementation, review and verification; Playwright for browser-level checks of every surface, including the deployed pair.

## 17. Learning

Significant.

## 18. AI value for my career

Yes.

---

## Attestations (confirmed by the submitter)

- At least the age of majority in Vietnam.
- From an eligible jurisdiction (Vietnam is not on the excluded list).
- Not an employee, representative or agent of the promotion entities.
