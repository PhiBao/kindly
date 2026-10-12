# The three-minute demo

For the PayPal AI Hackathon submission. Under three minutes, one memorable moment, every
claim visible on screen.

**Before you record**

```bash
./scripts/dev-server.sh --reset          # fresh demo studio, simulation mode
cd web && pnpm build && pnpm start       # http://localhost:3000
python3 scripts/smoke-demo.py            # confirm all 34 checks pass
```

Have two browser windows ready: the desk at `localhost:3000`, and a second window for the
client portal (you will copy the link from the invoice desk). Record at 1440×900, browser
zoom 110%, no bookmarks bar. Rehearse the agent-to-agent scenario once so the transcript is
in your head — but always reset before a take, so the recording starts from the same state a
judge will see.

---

## 0:00 — 0:20 · The hook

**On screen:** the desk, top of the page. The headline sits above the numbers.

> "Freelancers don't fear the work. They fear the ask."

Cut to the four invoices: Acme Studio 11 days late, Northwind Coffee 34 days, Brightline
Media 96 days and five reminders deep.

> "Fifty-nine percent of small businesses are waiting on invoices more than thirty days
> overdue. The average business is owed seventeen thousand dollars. But the thing that
> actually stops people collecting is that chasing a client is unbearable. So we took the ask
> away from the human and gave it to an agent that can *settle*."

## 0:20 — 0:50 · The mandate

**On screen:** scroll to the mandate panel, then open **Scenarios**.

> "Kindly acts inside limits you write in plain language — two contacts a week, never below
> seventy percent up front, no discounts without asking. The model never decides whether an
> action is allowed. A pure function does that, from your mandate, and it records the
> verdict either way."

Click **Agent-to-agent negotiation**. You land in the negotiation room.

## 0:50 — 1:40 · The work

**On screen:** the negotiation room, quiet for a beat while the transcript lands.

> "Acme Studio has an accounts-payable agent. So instead of another email, Kindly opens
> machine-to-machine talks — and both sides prove their authority with a signed mandate."

Point at the envelopes as you narrate:

- **Offer** — Kindly wants $1,680 up front.
- **Counter** — Acme's agent can only release $1,200 without a human.
- **The refusal** — "Refused their counter automatically: only 50% paid up front — below your
  70% floor."

> "That block is not a prompt. It is your limit, evaluated before any money could move.
> Kindly holds the line instead of conceding."

- **Their escalation** — Acme's agent cannot release $1,680 alone, so it asks its finance lead.

> "Both sides stop where a human should stop. Every envelope in this room shows *verified* —
> those are real Ed25519 signatures over canonical JSON."

## 1:40 — 2:15 · The wow

**On screen:** press **Their finance lead approves**.

> "Their finance lead approves the release. Kindly issues the deferred part as real PayPal
> invoices — two instalments, dated, sent."

Show the issued instalments (1042-P1, 1042-P2) appearing in the room, then open the invoice
desk and press the **PLAN ISSUED** stamp area.

## 2:15 — 2:45 · The client side, and the money

**On screen:** invoice desk → copy the client link → open it in the second window.

> "This is what the client sees: a signed, expiring link. No account. They read the letters,
> they can pay, and they can negotiate — in their own words."

Type into the box: *"could we do $575 now and the rest on the 20th?"* — press **Send**.

> "Kindly reads that as a plan request, answers in your voice, and — notice — the terms are
> clamped to your floor: $805 now, the rest in two instalments."

Press **Pay with PayPal**, then the **Pay part** button. The PAID stamp lands.

> "The payment runs through PayPal, the webhook comes back verified, and the ledger closes
> the loop."

## 2:45 — 3:00 · The payoff

**On screen:** the desk's proof panel — attribution and the trust column.

> "And it tells you the truth about what worked: *paid two days after the second reminder —
> eleven days earlier than this client's average*, or *paid on their own, Kindly takes no
> credit*. Everything the agent did, refused, or asked about is in one append-only ledger.
>
> You do the work. Kindly does the asking."

End card: **Kindly — the asking, handled** · demo URL · GitHub URL.

---

## Reliability notes for the recording

- **Reset before every take** (`Scenarios` → the reset happens automatically when the server
  is started with `--reset`; otherwise use the reset button in the scenarios panel).
- The agent-to-agent scenario is deterministic: their agent's policy is fixed, so the
  transcript will read the same way every time.
- If you record with a language model configured, drafts will differ in wording between
  takes. That is fine — the *terms*, the refusal and the amounts do not move. If you want
  byte-identical takes, record without `KINDLY_LLM_API_KEY` (templates), or accept the
  variation as proof that the model is doing the writing.
- The portal link is long. Copy it rather than typing it.
- Do not cut the three seconds after the PAID stamp: the thud is the emotional beat.
