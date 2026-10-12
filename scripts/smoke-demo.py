#!/usr/bin/env python3
"""Kindly end-to-end smoke test.

Runs the whole product loop against a live server (simulation mode by default):
agent-to-agent negotiation, the client's approval, real payments through the
webhook pipeline, a human payment plan, the instruction-injection defence,
quiet hours, and escalation.

    python3 scripts/smoke-demo.py [--base http://localhost:8787]

Exit code 0 means every check passed. The script is also the reproduction
recipe referenced by the README's verification section.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

PASS = "\033[32mPASS\033[0m"
FAIL = "\033[31mFAIL\033[0m"

failures: list[str] = []


def call(base: str, method: str, path: str, body: dict | None = None) -> dict:
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(
        f"{base}{path}",
        data=data,
        method=method,
        headers={"content-type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            payload = response.read().decode()
            return json.loads(payload) if payload else {}
    except urllib.error.HTTPError as error:  # surface API errors as data
        payload = error.read().decode()
        try:
            return json.loads(payload)
        except json.JSONDecodeError:
            return {"error": {"message": payload or str(error)}}


def check(name: str, condition: bool, detail: str = "") -> None:
    status = PASS if condition else FAIL
    print(f"  [{status}] {name}{(' — ' + detail) if detail else ''}")
    if not condition:
        failures.append(name)


def invoice_by_number(base: str, number: str) -> dict:
    bootstrap = call(base, "GET", "/api/bootstrap")
    for invoice in bootstrap.get("invoices", []):
        if invoice["number"] == number:
            return invoice
    raise SystemExit(f"invoice {number} not found (is the demo data seeded?)")


def invoice_detail(base: str, number: str) -> dict:
    invoice = invoice_by_number(base, number)
    return call(base, "GET", f"/api/invoices/{invoice['id']}")


def portal_token(detail: dict) -> str:
    return detail["portal_url"].split("/c/")[-1]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://localhost:8787")
    parser.add_argument("--keep", action="store_true", help="do not reset the data first")
    args = parser.parse_args()
    base = args.base.rstrip("/")

    print(f"\nKindly smoke test against {base}\n")

    health = call(base, "GET", "/api/health")
    if health.get("status") != "ok":
        print(f"server not healthy: {health}")
        return 2
    print(f"  mode={health['paypal_mode']}  model={health['language_model']}\n")

    if not args.keep:
        print("reset")
        call(base, "POST", "/api/demo/reset")

    # ── 1. seeded state ────────────────────────────────────────────────
    print("seeded state")
    bootstrap = call(base, "GET", "/api/bootstrap")
    numbers = {i["number"]: i for i in bootstrap["invoices"]}
    check("four demo invoices present", len(numbers) == 4, ", ".join(sorted(numbers)))
    check(
        "Acme invoice is 11 days overdue",
        numbers.get("1042", {}).get("days_overdue") == 11,
        f"got {numbers.get('1042', {}).get('days_overdue')}",
    )
    check(
        "Brightline has been contacted five times",
        invoice_detail(base, "1027")["negotiation"]["contact_count"] >= 5,
    )

    # ── 2. agent-to-agent negotiation ──────────────────────────────────
    print("\nagent-to-agent negotiation (Acme)")
    result = call(base, "POST", "/api/demo/scenario/agent_to_agent")
    session = result.get("session") or {}
    transcript = session.get("transcript", [])
    check("agent-to-agent session opened", bool(session), session.get("status", "missing"))
    check("six signed messages exchanged", len(transcript) >= 6, f"{len(transcript)} messages")
    check("every envelope verified", all(t["verified"] for t in transcript))
    kinds = [t["message"].get("type") for t in transcript]
    check("contains hello, offer and counter", {"hello", "offer", "counter"} <= set(kinds), str(kinds))
    check(
        "the buyer is asked for human approval",
        session.get("status") == "awaiting_buyer_approval",
        session.get("status", "?"),
    )
    ledger = call(base, "GET", "/api/ledger?limit=200")
    blocked = [e for e in ledger if e.get("kind") == "policy_blocked"]
    check(
        "Kindly refused the sub-floor counter automatically",
        any("below your 70% floor" in e.get("summary", "") for e in blocked),
        f"{len(blocked)} block(s) recorded",
    )

    # ── 3. the client approves, plan is issued ─────────────────────────
    print("\nclient approval and plan issue")
    approved = call(base, "POST", f"/api/a2a/{session['id']}/buyer-approval")
    check("negotiation agreed", approved.get("status") == "plan_issued", approved.get("status", "?"))
    instalments = [
        i for i in call(base, "GET", "/api/bootstrap")["invoices"] if str(i["number"]).startswith("1042-P")
    ]
    check("PayPal invoices issued for the deferred part", len(instalments) == 2, f"{len(instalments)}")
    detail = invoice_detail(base, "1042")
    check(
        "client can see the agreed plan on their portal",
        len(call(base, "GET", f"/api/portal/{portal_token(detail)}")["messages"]) >= 1,
    )

    # ── 4. the client pays through the portal ──────────────────────────
    print("\nclient pays 70% through the portal")
    token = portal_token(detail)
    order = call(base, "POST", f"/api/portal/{token}/order", {"amount_minor": 168_000})
    check("a PayPal order was created", bool(order.get("order_id")), order.get("order_id", "?"))
    capture = call(base, "POST", f"/api/portal/{token}/capture", {"order_id": order["order_id"]})
    check("capture processed through the webhook pipeline", capture.get("captured") is True, json.dumps(capture)[:120])
    detail = invoice_detail(base, "1042")
    check(
        "invoice is partly paid",
        detail["invoice"]["status_label"] == "Partly paid",
        detail["invoice"]["outstanding"],
    )
    check("outstanding dropped to $720.00", detail["invoice"]["outstanding"] == "$720.00")

    # ── 5. an instalment is paid, invoice settles ──────────────────────
    print("\nthe client pays the first instalment")
    paid = call(base, "POST", "/api/demo/scenario/plan_instalment_paid")
    check("instalment settled", paid.get("events_processed", 0) >= 1, json.dumps(paid)[:140])
    detail = invoice_detail(base, "1042")
    check(
        "agreement settled against the parent invoice",
        detail["invoice"]["outstanding"] in ("$360.00", "$0.00"),
        detail["invoice"]["outstanding"],
    )
    ledger = call(base, "GET", "/api/ledger?limit=200")
    check(
        "attribution recorded once settled",
        any("Attribution" in e.get("summary", "") for e in ledger),
    )

    # ── 6. the human conversation path ─────────────────────────────────
    print("\na client asks to split the payment (Northwind)")
    plan = call(base, "POST", "/api/demo/scenario/human_plan")
    latest = plan.get("latest_action") or {}
    check(
        "Kindly offered compliant terms",
        latest.get("kind") == "offer_terms" and latest.get("status") == "executed",
        f"{latest.get('kind')} / {latest.get('status')}",
    )
    detail = invoice_detail(base, "1039")
    offers = [m for m in detail["messages"] if "plan" in (m["subject"] or "").lower()]
    check("the offer is in the client thread", bool(offers), f"{len(offers)} message(s)")

    print("\nthe client replies in their portal")
    reply = call(
        base,
        "POST",
        f"/api/portal/{portal_token(detail)}/message",
        {"body": "Thanks for the update. Could we do $575 now and $575 on the 20th?"},
    )
    check(
        "reply understood as a plan request",
        "plan" in reply.get("intent", ""),
        reply.get("intent", "?"),
    )

    # ── 7. the instruction-injection defence ───────────────────────────
    print("\na reply tries to instruct the agent (Acme)")
    injection = call(base, "POST", "/api/demo/scenario/injection")
    check("the money was not changed", injection.get("money_unchanged") is True)
    check(
        "outstanding balance is identical before and after",
        injection.get("outstanding_before") == injection.get("outstanding_after"),
        f"{injection.get('outstanding_before')} -> {injection.get('outstanding_after')}",
    )
    check("still outstanding", injection.get("still_outstanding") not in (None, "$0.00"), injection.get("still_outstanding", "?"))
    check("a security event was recorded", bool(injection.get("ledger")))

    # ── 8. quiet hours ─────────────────────────────────────────────────
    print("\nquiet hours")
    quiet = call(base, "POST", "/api/demo/scenario/quiet_hours")
    at_3am = quiet.get("at_0300", {}).get("decision", {}).get("decision")
    at_11am = quiet.get("at_1100", {}).get("decision", {}).get("decision")
    check("03:00 is blocked", at_3am == "block", str(at_3am))
    check("11:00 is allowed", at_11am == "allow", str(at_11am))

    # ── 9. escalation ──────────────────────────────────────────────────
    print("\nan ignored invoice escalates (Brightline)")
    escalation = call(base, "POST", "/api/demo/scenario/escalation")
    approvals = escalation.get("approvals", [])
    check("an escalation is waiting for the owner", bool(approvals), f"{len(approvals)} pending")
    if approvals:
        decision = call(base, f"POST", f"/api/actions/{approvals[0]['id']}/decision", {"approve": False})
        check("the owner can decline", decision.get("status") == "rejected", decision.get("status", "?"))

    # ── 10. the audit trail ────────────────────────────────────────────
    print("\naudit trail")
    ledger = call(base, "GET", "/api/ledger?limit=500")
    trust_kinds = {
        "policy_blocked",
        "policy_approval_requested",
        "policy_approval_granted",
        "policy_approval_rejected",
        "security_event",
        "mandate_updated",
    }
    trust_events = [e for e in ledger if e.get("kind") in trust_kinds]
    check("trust events are in the ledger", len(trust_events) >= 4, f"{len(trust_events)} events")
    check("every ledger row has an actor and a summary", all(e.get("actor") and e.get("summary") for e in ledger))
    events = call(base, "GET", "/api/bootstrap")["paypal_events"]
    check("PayPal webhooks received and verified", len(events) >= 1, f"{len(events)} events")
    check("all webhooks verified", all(e["verified"] for e in events))

    print()
    if failures:
        print(f"\033[31m{len(failures)} check(s) failed:\033[0m " + "; ".join(failures))
        return 1
    print("\033[32mAll checks passed.\033[0m")
    return 0


if __name__ == "__main__":
    sys.exit(main())
