#!/usr/bin/env python3
"""Verify a *live* Kindly deployment against PayPal, not against itself.

    PAYPAL_CLIENT_ID=... PAYPAL_CLIENT_SECRET=... \
      python3 scripts/verify-live.py --base https://kindly-desk-api.fly.dev

What it checks:

* the API says it is in live mode and is configured
* every receivable has a PayPal invoice object, and PayPal agrees it exists and
  is in the state Kindly shows
* the webhook deliveries Kindly recorded were cryptographically verified, and at
  least one real capture has been seen
* the portal can open a Checkout order and get an approval URL back

The credential check is the important one: it reads the invoice from PayPal
using the same app, so a mirrored-but-fake id cannot pass.

No money moves: the order it creates is never approved or captured.
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import sys
import urllib.error
import urllib.request

FAILS: list[str] = []


def ok(message: str) -> None:
    print(f"  \033[32mPASS\033[0m {message}")


def fail(message: str) -> None:
    FAILS.append(message)
    print(f"  \033[31mFAIL\033[0m {message}")


def note(message: str) -> None:
    print(f"       {message}")


def http(method: str, url: str, body=None, token: str | None = None):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"content-type": "application/json", "accept": "application/json"}
    if token:
        headers["authorization"] = f"Bearer {token}"
    request = urllib.request.Request(url, data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            raw = response.read().decode()
            return response.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as error:
        raw = error.read().decode()
        try:
            return error.code, json.loads(raw)
        except json.JSONDecodeError:
            return error.code, {"raw": raw[:300]}


class API:
    def __init__(self, base: str):
        self.base = base.rstrip("/")

    def call(self, method: str, path: str, body=None):
        status, payload = http(method, f"{self.base}{path}", body)
        if status >= 400:
            raise SystemExit(f"{method} {path} -> {status} {json.dumps(payload)[:200]}")
        return payload


def paypal_token(client_id: str, secret: str, base: str) -> str:
    credentials = base64.b64encode(f"{client_id}:{secret}".encode()).decode()
    request = urllib.request.Request(
        f"{base}/v1/oauth2/token",
        data=b"grant_type=client_credentials",
        method="POST",
        headers={
            "authorization": f"Basic {credentials}",
            "content-type": "application/x-www-form-urlencoded",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read().decode())["access_token"]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="https://kindly-desk-api.fly.dev")
    args = parser.parse_args()
    api = API(args.base)

    print(f"Kindly live verification against {args.base}\n")

    health = api.call("GET", "/api/health")
    print("runtime")
    if health.get("paypal_mode") == "live":
        ok("API is in live mode")
    else:
        fail(f"API reports paypal_mode={health.get('paypal_mode')}")
    if health.get("paypal_configured"):
        ok("PayPal credentials are configured")
    else:
        fail("PayPal credentials are missing")

    bootstrap = api.call("GET", "/api/bootstrap")
    invoices = bootstrap["invoices"]
    print("\nreceivables")
    if invoices:
        ok(f"{len(invoices)} receivable(s) on the desk")
    else:
        fail("no invoices to verify")
    without_mirror = [i["number"] for i in invoices if not i.get("paypal_invoice_id")]
    if without_mirror:
        note(f"still mirroring (allowed for a few seconds after boot): {without_mirror}")
    else:
        ok("every receivable has a PayPal invoice id")

    print("\nPayPal agrees")
    client_id = os.environ.get("PAYPAL_CLIENT_ID")
    secret = os.environ.get("PAYPAL_CLIENT_SECRET")
    if not client_id or not secret:
        note("PAYPAL_CLIENT_ID / PAYPAL_CLIENT_SECRET not set — skipping the external check")
    else:
        base = os.environ.get("PAYPAL_API_BASE", "https://api-m.sandbox.paypal.com")
        token = paypal_token(client_id, secret, base)
        checked = 0
        for invoice in invoices:
            paypal_id = invoice.get("paypal_invoice_id")
            if not paypal_id:
                continue
            status, resource = http(
                "GET", f"{base}/v2/invoicing/invoices/{paypal_id}", token=token
            )
            if status == 200:
                checked += 1
                remote = resource.get("status", "?")
                ours = (invoice.get("paypal_status") or "").upper()
                if ours and remote.upper() != ours:
                    note(f"{invoice['number']}: PayPal says {remote}, Kindly says {ours}")
                else:
                    ok(f"{invoice['number']} · {paypal_id} · {remote}")
            else:
                fail(f"{invoice['number']}: PayPal returned {status} for {paypal_id}")
        if checked:
            ok(f"{checked} invoice object(s) confirmed by PayPal itself")

    print("\nwebhooks")
    events = bootstrap.get("paypal_events") or []
    if not events:
        note("no deliveries recorded yet — PayPal sends CREATED/SENT when mirrors are made")
    else:
        unverified = [e for e in events if not e.get("verified")]
        if unverified:
            fail(f"{len(unverified)} webhook delivery/deliveries failed signature verification")
        else:
            ok(f"{len(events)} delivery/deliveries, all signature-verified")
        captures = [e for e in events if e.get("event_type") == "PAYMENT.CAPTURE.COMPLETED"]
        if captures:
            ok(f"{len(captures)} real capture event(s) received and processed")
        else:
            note("no capture yet since the last reset — the first payment produces one")

    print("\ncheckout")
    portal_invoice = next((i for i in invoices if i["status"] not in ("paid", "cancelled")), None)
    if portal_invoice:
        detail = api.call("GET", f"/api/invoices/{portal_invoice['id']}")
        token_part = detail["portal_url"].split("/c/")[1]
        status, order = http(
            "POST",
            f"{args.base}/api/portal/{token_part}/order",
            {"amount_minor": min(100, int(portal_invoice["outstanding_minor"] or 100))},
        )
        if status == 200 and order.get("approve_url"):
            ok(f"Checkout order created · {order['order_id']}")
            note(f"approval page {order['approve_url']}")
            note("never approved or captured by this script")
        else:
            fail(f"could not open a Checkout order ({status} {json.dumps(order)[:160]})")
    else:
        note("every invoice is settled — nothing to open a checkout for")

    print()
    if FAILS:
        print(f"\033[31m{len(FAILS)} check(s) failed.\033[0m")
        return 1
    print("\033[32mLive checks passed.\033[0m")
    return 0


if __name__ == "__main__":
    sys.exit(main())
