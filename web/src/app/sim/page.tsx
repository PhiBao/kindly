"use client";

import Link from "next/link";
import { StampK, Wordmark } from "@/components/primitives";
import { useBootstrap } from "@/lib/use-bootstrap";

const SIM_URL = "https://kindly-desk-sim.vercel.app";
const LIVE_URL = "https://kindly-desk-web.vercel.app";

/**
 * Two ways to run the same product.
 *
 * The main desk talks to PayPal's sandbox and takes real (test) money through
 * Checkout. This page explains that difference and opens the simulation, where
 * the same code runs against an in-process PayPal and a payment completes
 * without an account.
 */
export default function SimPage() {
  const { data } = useBootstrap();
  const live = data?.owner.paypal_mode_raw === "live";
  const here = live ? LIVE_URL : SIM_URL;
  const other = live ? SIM_URL : LIVE_URL;

  return (
    <div className="mx-auto w-full max-w-[1000px] px-5 pb-24 pt-7 sm:px-8">
      <header className="rule-heavy-b flex flex-wrap items-end justify-between gap-4 pb-4">
        <Link href="/" className="flex items-end gap-3">
          <StampK size={30} />
          <Wordmark size={23} />
        </Link>
        <span className="eyebrow pb-1">
          currently: {live ? "PayPal sandbox" : "simulation"}
        </span>
      </header>

      <h1 className="display mt-7 max-w-[26ch] text-[2.4rem] leading-[0.98] sm:text-[3rem]">
        Two ways to run this desk.
      </h1>
      <p className="mt-4 max-w-[62ch] text-[0.9375rem] leading-relaxed text-[var(--color-ink-2)]">
        The product is the same in both. The difference is who holds the money: PayPal&apos;s
        sandbox, or a PayPal that lives inside the server.
      </p>

      <div className="mt-9 grid gap-6 sm:grid-cols-2">
        <section className={`paper-edge p-5 ${live ? "outline outline-1 outline-[var(--color-ink)]" : ""}`}>
          <div className="eyebrow">Live · PayPal sandbox</div>
          <h2 className="display mt-1.5 text-[1.4rem] leading-tight">Real API, test money.</h2>
          <p className="mt-2 text-[0.8125rem] leading-relaxed text-[var(--color-ink-2)]">
            Kindly calls PayPal&apos;s real APIs: it creates invoice objects, opens Checkout
            orders, verifies signed webhooks, and records the capture. The money is sandbox
            money — PayPal&apos;s test rail. Nothing reaches a bank.
          </p>
          <dl className="mt-4 space-y-1.5">
            <div className="flex items-baseline justify-between gap-3 rule-t pt-1.5">
              <dt className="mono-xs">pay with</dt>
              <dd className="num text-[0.8125rem]">test card, no account</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">card number</dt>
              <dd className="num text-[0.8125rem]">4111 1111 1111 1111</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">expiry / CVC</dt>
              <dd className="num text-[0.8125rem]">any future date · 123</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">billing address</dt>
              <dd className="num text-[0.8125rem]">any US address</dd>
            </div>
          </dl>
          <p className="mono-xs mt-3 rule-t pt-2">
            At PayPal&apos;s checkout, choose “Pay with debit or credit card”, then “Pay now as
            guest”. You are returned here and the invoice settles.
          </p>
          <a className="btn btn--solid mt-4" href={LIVE_URL}>
            "Open the live desk"
          </a>
        </section>

        <section className={`paper-edge p-5 ${!live ? "outline outline-1 outline-[var(--color-ink)]" : ""}`}>
          <div className="eyebrow">Simulation · no network</div>
          <h2 className="display mt-1.5 text-[1.4rem] leading-tight">Same product, no PayPal.</h2>
          <p className="mt-2 text-[0.8125rem] leading-relaxed text-[var(--color-ink-2)]">
            A faithful in-process PayPal implements the same interface, signs the same webhook
            deliveries, and goes through the same verification and ledger. Nothing leaves the
            server. Payments complete with one click.
          </p>
          <dl className="mt-4 space-y-1.5">
            <div className="flex items-baseline justify-between gap-3 rule-t pt-1.5">
              <dt className="mono-xs">pay with</dt>
              <dd className="num text-[0.8125rem]">one click</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">credentials</dt>
              <dd className="num text-[0.8125rem]">none</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">deterministic</dt>
              <dd className="num text-[0.8125rem]">yes</dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="mono-xs">money</dt>
              <dd className="num text-[0.8125rem]">none</dd>
            </div>
          </dl>
          <p className="mono-xs mt-3 rule-t pt-2">
            Every screen is labelled “simulated PayPal”. Use it when the sandbox is having a bad
            day, or when you want the payment step to take one click instead of five.
          </p>
          <a className="btn mt-4" href={other}>
            {live ? "Open the simulation" : "Open the live desk"}
          </a>
        </section>
      </div>

      <section className="mt-9 rule-t pt-4">
        <div className="eyebrow">What is identical</div>
        <p className="mt-2 max-w-[70ch] text-[0.8125rem] leading-relaxed text-[var(--color-ink-2)]">
          The mandate and policy engine, the agent-to-agent negotiation, the ledger, attribution,
          the portal, and the refusal paths. The only swap is the PayPal client — which is why the
          simulator is a legitimate way to judge the product, and not a mock of it.
        </p>
        <p className="mono-xs mt-3">
          {here.replace("https://", "")} · API {live ? "live" : "simulation"}
        </p>
      </section>
    </div>
  );
}
