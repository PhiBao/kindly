#!/usr/bin/env python3
"""Generate Kindly's diagrams.

Three diagrams, each answering a question a judge actually has:

  1. architecture.svg — what runs where, and how PayPal gets in and out
  2. decision-loop.svg — who decides what the agent may do (the policy gate)
  3. settlement.svg   — how an agreement turns into money, message by message

They are generated rather than hand-drawn so the geometry stays on a 4px grid,
arrows stay orthogonal, and labels keep their masks. Brand tokens are Kindly's
own (paper, ink, oxblood, settled green) — not the diagram skill's defaults.

    python3 docs/diagrams/build.py

Output: docs/diagrams/*.svg plus an index.html gallery.
"""

from __future__ import annotations

import html
import pathlib

# ── brand tokens (mirrors web/src/app/globals.css) ──────────────────────────
PAPER = "#f7f2e8"
PAPER_2 = "#fdfaf3"
PAPER_3 = "#efe6d4"
INK = "#16181d"
INK_2 = "#454b59"
MUTED = "#737a8a"
RULE = "#ddd3c0"
RULE_2 = "#c6b8a0"
STAMP = "#a32b1e"
SETTLED = "#1d5b46"
AMBER = "#a06d12"

SANS = "Inter Tight, Inter, -apple-system, system-ui, sans-serif"
MONO = "IBM Plex Mono, ui-monospace, SFMono-Regular, monospace"
SERIF = "Instrument Serif, Iowan Old Style, Georgia, serif"

GRID = 4


def snap(value: float) -> float:
    return round(value / GRID) * GRID


class Svg:
    """Tiny drawing surface: boxes, orthogonal arrows, masked labels."""

    def __init__(self, width: int, height: int, slug: str, title: str, desc: str):
        self.width = width
        self.height = height
        self.slug = slug
        self.title = title
        self.desc = desc
        self.arrows: list[str] = []
        self.nodes: list[str] = []
        self.masks: list[str] = []
        self.texts: list[str] = []

    # ── primitives ─────────────────────────────────────────────────────────
    def box(
        self,
        x: float,
        y: float,
        w: float,
        h: float,
        name: str,
        sublabels: list[str] | None = None,
        *,
        tone: str = "plain",
        tag: str | None = None,
        focal: bool = False,
        mono_sublabels: bool = True,
    ) -> tuple[float, float]:
        x, y, w, h = snap(x), snap(y), snap(w), snap(h)
        fills = {
            "plain": PAPER_2,
            "store": PAPER_3,
            "external": "none",
            "input": "none",
            "core": "#ffffff",
        }
        stroke = INK if focal else RULE_2 if tone in ("external", "input") else RULE
        stroke_width = 1.6 if focal else 1.1
        dash = ' stroke-dasharray="5,4"' if tone in ("external", "input") else ""
        if focal:
            stroke = STAMP

        self.nodes.append(
            f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="6" fill="{fills[tone]}" '
            f'stroke="{stroke}" stroke-width="{stroke_width}"{dash}/>'
        )
        if tag:
            self.nodes.append(
                f'<rect x="{x}" y="{y}" width="{snap(len(tag) * 6.0 + 14)}" height="16" rx="2" '
                f'fill="{PAPER_3}" stroke="{RULE}" stroke-width="1"/>'
            )
            self.texts.append(
                f'<text x="{x + 7}" y="{y + 11.5}" font-family="{MONO}" font-size="9" '
                f'letter-spacing="1.1" fill="{MUTED}">{html.escape(tag.upper())}</text>'
            )

        text_y = y + (30 if tag else 24)
        self.texts.append(
            f'<text x="{x + 16}" y="{text_y}" font-family="{SANS}" font-size="14.5" font-weight="600" '
            f'fill="{INK}">{html.escape(name)}</text>'
        )
        for index, sub in enumerate(sublabels or []):
            self.texts.append(
                f'<text x="{x + 16}" y="{text_y + 16 + index * 13}" font-family="'
                f'{MONO if mono_sublabels else SANS}" font-size="10.5" fill="{MUTED}">'
                f"{html.escape(sub)}</text>"
            )
        return x, y

    def arrow(
        self,
        x1: float,
        y1: float,
        x2: float,
        y2: float,
        *,
        label: str | None = None,
        tone: str = "muted",
        dashed: bool = False,
        elbow: str | None = None,
        radius: int = 8,
    ) -> None:
        """Orthogonal connector. `elbow` in {h-then-v, v-then-h}."""
        colors = {"muted": MUTED, "accent": STAMP, "link": INK_2, "settled": SETTLED}
        color = colors[tone]
        dash = ' stroke-dasharray="5,4"' if dashed else ""
        marker = {"accent": "url(#arrow-accent)", "settled": "url(#arrow-settled)"}.get(
            tone, "url(#arrow)"
        )

        x1, y1, x2, y2 = snap(x1), snap(y1), snap(x2), snap(y2)
        if x1 == x2 or y1 == y2:
            path = f"M {x1} {y1} L {x2} {y2}"
        elif elbow in ("h-then-v", "v-then-h"):
            if elbow == "h-then-v":
                path = (
                    f"M {x1} {y1} L {x2} {y1} "
                    f"Q {x2} {y1} {x2} {y1 + (radius if y2 > y1 else -radius)} "
                    f"L {x2} {y2}"
                )
                path = (
                    f"M {x1} {y1} H {snap(x2 - (radius if x2 > x1 else -radius))} "
                    f"Q {x2} {y1} {x2} {snap(y1 + (radius if y2 > y1 else -radius))} V {y2}"
                )
            else:
                path = (
                    f"M {x1} {y1} V {snap(y2 - (radius if y2 > y1 else -radius))} "
                    f"Q {x1} {y2} {snap(x1 + (radius if x2 > x1 else -radius))} {y2} H {x2}"
                )
        else:  # default: horizontal then vertical, corner at (x2, y1)
            path = (
                f"M {x1} {y1} H {snap(x2 - (radius if x2 > x1 else -radius))} "
                f"Q {x2} {y1} {x2} {snap(y1 + (radius if y2 > y1 else -radius))} V {y2}"
            )

        self.arrows.append(
            f'<path d="{path}" fill="none" stroke="{color}" stroke-width="1.3"{dash} '
            f'marker-end="{marker}"/>'
        )

        if label:
            # Label sits beside the vertical run (or above the horizontal one),
            # always on an opaque mask so nothing shows through.
            if x1 == x2 or abs(x2 - x1) < 60:
                lx, ly = x1, snap((y1 + y2) / 2)
                w = len(label) * 6.0 + 12
                self.masks.append(f'<rect x="{lx - w / 2}" y="{ly - 8}" width="{w}" height="15" fill="{PAPER}"/>')
                self.texts.append(
                    f'<text x="{lx}" y="{ly + 3}" text-anchor="middle" font-family="{MONO}" '
                    f'font-size="9.5" letter-spacing="0.8" fill="{color}">{html.escape(label.upper())}</text>'
                )
            else:
                lx, ly = snap((x1 + x2) / 2), y1 - 7
                w = len(label) * 6.0 + 12
                self.masks.append(f'<rect x="{lx - w / 2}" y="{ly - 8}" width="{w}" height="15" fill="{PAPER}"/>')
                self.texts.append(
                    f'<text x="{lx}" y="{ly + 3}" text-anchor="middle" font-family="{MONO}" '
                    f'font-size="9.5" letter-spacing="0.8" fill="{color}">{html.escape(label.upper())}</text>'
                )

    def route(
        self,
        points: list[tuple[float, float]],
        *,
        label: str | None = None,
        tone: str = "muted",
        dashed: bool = False,
        radius: int = 8,
    ) -> None:
        """Orthogonal polyline with rounded corners (two corners maximum)."""
        colors = {"muted": MUTED, "accent": STAMP, "link": INK_2, "settled": SETTLED}
        color = colors[tone]
        dash = ' stroke-dasharray="5,4"' if dashed else False
        pts = [(snap(x), snap(y)) for x, y in points]
        d = f"M {pts[0][0]} {pts[0][1]}"
        for index in range(1, len(pts)):
            x, y = pts[index]
            px, py = pts[index - 1]
            if index < len(pts) - 1:
                nx, ny = pts[index + 1]
                if px == x:  # arriving vertically
                    d += f" L {x} {y - radius if py < y else y + radius}"
                    d += f" L {x - radius if nx < x else x + radius} {y}"
                else:  # arriving horizontally
                    d += f" L {x - radius if px < x else x + radius} {y}"
                    d += f" L {x} {y - radius if ny < y else y + radius}"
            else:
                d += f" L {x} {y}"
        self.arrows.append(
            f'<path d="{d}" fill="none" stroke="{color}" stroke-width="1.3"'
            f'{"" if dashed is False else " stroke-dasharray=\"5,4\""} marker-end="url(#arrow)"/>'
        )
        if label:
            mid = points[len(points) // 2]
            w = len(label) * 6.0 + 12
            self.masks.append(
                f'<rect x="{snap(mid[0]) - w / 2}" y="{snap(mid[1]) - 20}" width="{w}" height="15" fill="{PAPER}"/>'
            )
            self.texts.append(
                f'<text x="{snap(mid[0])}" y="{snap(mid[1]) - 9}" text-anchor="middle" '
                f'font-family="{MONO}" font-size="9.5" letter-spacing="0.8" fill="{color}">'
                f"{html.escape(label.upper())}</text>"
            )

    def note(self, x: float, y: float, text: str, *, width: int = 360) -> None:
        self.texts.append(
            f'<text x="{snap(x)}" y="{snap(y)}" font-family="{SERIF}" font-style="italic" '
            f'font-size="15" fill="{INK_2}">{html.escape(text)}</text>'
        )
        self.arrows.append("")

    def legend(self, y: float, items: list[tuple[str, str]]) -> None:
        x = 40
        for label, color in items:
            self.nodes.append(
                f'<rect x="{x}" y="{snap(y) - 9}" width="26" height="0" fill="none"/>'
            )
            self.arrows.append(
                f'<line x1="{x}" y1="{snap(y) - 4}" x2="{x + 22}" y2="{snap(y) - 4}" '
                f'stroke="{color}" stroke-width="1.4" marker-end="url(#arrow)"/>'
                if color != "shape"
                else ""
            )
            self.texts.append(
                f'<text x="{x + 30}" y="{snap(y)}" font-family="{MONO}" font-size="9.5" '
                f'letter-spacing="0.8" fill="{MUTED}">{html.escape(label.upper())}</text>'
            )
            x += 40 + len(label) * 6.6

    # ── output ─────────────────────────────────────────────────────────────
    def render(self) -> str:
        defs = f"""<defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="{MUTED}"/>
    </marker>
    <marker id="arrow-accent" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="{STAMP}"/>
    </marker>
    <marker id="arrow-settled" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 0 L 10 5 L 0 10 z" fill="{SETTLED}"/>
    </marker>
  </defs>"""

        return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" width="{self.width}" height="{self.height}" role="img" aria-labelledby="{self.slug}-title {self.slug}-desc">
  <title id="{self.slug}-title">{html.escape(self.title)}</title>
  <desc id="{self.slug}-desc">{html.escape(self.desc)}</desc>
  {defs}
  <rect width="{self.width}" height="{self.height}" fill="{PAPER}"/>
  {''.join(self.arrows)}
  {''.join(self.nodes)}
  {''.join(self.masks)}
  {''.join(self.texts)}
</svg>
"""


# ── 1. architecture ─────────────────────────────────────────────────────────
def architecture() -> Svg:
    svg = Svg(
        1040,
        640,
        "architecture",
        "Kindly architecture",
        "Two surfaces over a Rust server: the policy engine gates every action, "
        "PayPal is called through Invoicing and Orders APIs, and webhook deliveries "
        "return verified payments into an append-only ledger.",
    )
    svg.texts.append(
        f'<text x="40" y="46" font-family="{SERIF}" font-size="26" fill="{INK}">'
        "What runs where</text>"
    )
    svg.texts.append(
        f'<text x="40" y="68" font-family="{SANS}" font-size="12" fill="{MUTED}">'
        "One Rust server, two surfaces, and PayPal on both the way out and the way back.</text>"
    )

    # col A — surfaces
    svg.box(40, 120, 240, 90, "Owner's desk", ["agent trail · approvals", "mandate · money view"], tone="input", tag="browser")
    svg.box(40, 250, 240, 90, "Client portal", ["signed, expiring link", "pay · propose a plan"], tone="input", tag="public")

    # col B — the server
    svg.box(330, 120, 300, 220, "Kindly server", ["axum · tokio · sqlx", "api · sse · scheduler", "webhook intake"], tone="core", tag="rust")

    # col C — the core and the money
    svg.box(700, 120, 300, 96, "Policy engine", ["your mandate, limit by limit", "allow · ask · block"], focal=True, tag="deterministic")
    svg.box(700, 250, 300, 90, "Agent layer", ["planner · composer", "reply understanding"], tone="core", tag="proposes only")

    # row 3
    svg.box(40, 400, 240, 96, "Ledger", ["append-only audit", "invoices · actions · events"], tone="store", tag="sqlite")
    svg.box(330, 400, 300, 96, "PayPal APIs", ["invoicing v2 · orders", "captures · reminders"], tone="external", tag="rest")
    svg.box(700, 400, 300, 96, "Language model", ["compose · understand terms"], tone="external", tag="optional")

    svg.arrow(280, 155, 330, 155, label="http")
    svg.arrow(330, 190, 280, 190, label="sse", tone="link")
    svg.arrow(280, 295, 330, 295, label="pay")
    svg.arrow(630, 155, 700, 155, label="evaluate", tone="accent")
    svg.arrow(630, 295, 700, 295, label="plan")
    svg.arrow(850, 250, 850, 216, label="proposes", dashed=True)
    svg.arrow(430, 340, 430, 400, label="act")
    svg.arrow(560, 400, 560, 340, label="delivers", tone="settled")
    svg.arrow(850, 340, 850, 400, label="composes", dashed=True)
    svg.route([(380, 340), (380, 372), (160, 372), (160, 400)], label="append")

    svg.texts.append(
        f'<text x="40" y="556" font-family="{SERIF}" font-style="italic" font-size="15" fill="{INK_2}">'
        "PayPal is the only way money moves, and the only writer of payment records Kindly trusts.</text>"
    )
    svg.texts.append(
        f'<text x="40" y="600" font-family="{MONO}" font-size="9.5" letter-spacing="0.8" fill="{MUTED}">'
        "SOLID = REQUEST PATH · DASHED = ASYNCHRONOUS OR OPTIONAL · GREEN = MONEY MOVED</text>"
    )
    return svg


# ── 2. decision loop ────────────────────────────────────────────────────────
def decision_loop() -> Svg:
    svg = Svg(
        1040,
        600,
        "decision-loop",
        "Kindly's decision loop",
        "A client message is understood, turned into one proposal, judged against "
        "the owner's mandate, and then executed, queued for the owner, or blocked — "
        "with all three outcomes written to the ledger.",
    )
    svg.texts.append(
        f'<text x="40" y="46" font-family="{SERIF}" font-size="26" fill="{INK}">'
        "Who decides what the agent may do</text>"
    )
    svg.texts.append(
        f'<text x="40" y="68" font-family="{SANS}" font-size="12" fill="{MUTED}">'
        "The model proposes. The mandate disposes. Every verdict is recorded.</text>"
    )

    svg.box(40, 140, 200, 84, "A client replies", ["letter or agent message"], tone="input", tag="input")
    svg.box(280, 140, 200, 84, "Understanding", ["intent · injection scan"], tone="core", tag="model + rules")
    svg.box(520, 140, 200, 84, "Proposal", ["one concrete action"], tone="core", tag="planner")
    svg.box(760, 140, 240, 84, "Policy engine", ["your mandate, limit by limit"], focal=True, tag="the gate")

    svg.box(760, 300, 240, 78, "Allowed", ["inside every limit"])
    svg.box(760, 392, 240, 78, "Waiting for you", ["a limit would be crossed"])
    svg.box(760, 484, 240, 78, "Blocked", ["refused, with the reason"])

    svg.box(40, 300, 240, 262, "The ledger", ["append-only · shows", "all three outcomes", "", "the owner can read", "every verdict later"], tone="store", tag="history")

    svg.arrow(240, 182, 280, 182)
    svg.arrow(480, 182, 520, 182)
    svg.arrow(720, 182, 760, 182, label="evaluate", tone="accent")
    svg.arrow(880, 224, 880, 300, label="allow", tone="accent")
    svg.arrow(880, 378, 880, 392, label="needs you", dashed=True)
    svg.arrow(880, 470, 880, 484, label="block", tone="accent")

    svg.arrow(760, 339, 280, 339, label="executed · recorded", tone="settled")
    svg.arrow(760, 431, 280, 431, label="queued · recorded", dashed=True)
    svg.arrow(760, 523, 280, 523, label="refused · recorded", tone="muted")

    svg.texts.append(
        f'<text x="40" y="592" font-family="{SERIF}" font-style="italic" font-size="15" fill="{INK_2}">'
        "Instruction attempts from clients land here too — read as data, never obeyed.</text>"
    )
    return svg


# ── 3. settlement sequence ──────────────────────────────────────────────────
def settlement() -> Svg:
    svg = Svg(
        1040,
        600,
        "settlement",
        "How an agreement becomes money",
        "Sequence from the first reminder to a captured PayPal payment: offers, "
        "counters, human approvals, issued instalments, and the webhook that closes "
        "the loop in the ledger.",
    )
    svg.texts.append(
        f'<text x="40" y="46" font-family="{SERIF}" font-size="26" fill="{INK}">'
        "From a late invoice to money in the account</text>"
    )
    svg.texts.append(
        f'<text x="40" y="68" font-family="{SANS}" font-size="12" fill="{MUTED}">'
        "The same path whether the client is a person replying by letter or an agent answering with terms.</text>"
    )

    lanes = [("Owner", 150), ("Kindly", 400), ("Client", 660), ("PayPal", 920)]
    for label, x in lanes:
        svg.texts.append(
            f'<text x="{x}" y="112" text-anchor="middle" font-family="{MONO}" font-size="10" '
            f'letter-spacing="1" fill="{MUTED}">{label.upper()}</text>'
        )
        svg.arrows.append(
            f'<line x1="{x}" y1="128" x2="{x}" y2="520" stroke="{RULE}" stroke-width="1" '
            f'stroke-dasharray="2,4"/>'
        )

    steps = [
        (150, 400, "reminder + pay link", "muted", False, 160),
        (660, 400, "reply asks to split", "muted", False, 204),
        (400, 660, "terms inside your limits", "accent", False, 248),
        (660, 400, "accepts (signed if an agent)", "muted", False, 292),
        (400, 920, "issues instalment invoices", "muted", False, 336),
        (660, 920, "pays the first part", "settled", False, 380),
        (920, 400, "webhook: payment captured", "settled", True, 424),
        (400, 150, "ledger + attribution", "settled", False, 468),
    ]
    for x1, x2, label, tone, dashed, y in steps:
        svg.arrow(x1, y, x2, y, label=label, tone=tone, dashed=dashed)

    svg.texts.append(
        f'<text x="40" y="556" font-family="{SERIF}" font-style="italic" font-size="15" fill="{INK_2}">'
        "Human approval sits at the two places where the money is unusual: an owner exception, "
        "or a client release above their own limit.</text>"
    )
    svg.texts.append(
        f'<text x="40" y="588" font-family="{MONO}" font-size="9.5" letter-spacing="0.8" fill="{MUTED}">'
        "SOLID = MESSAGE · DASHED = ASYNCHRONOUS DELIVERY · GREEN = MONEY MOVED</text>"
    )
    return svg


def main() -> None:
    here = pathlib.Path(__file__).parent
    diagrams = {
        "architecture": architecture(),
        "decision-loop": decision_loop(),
        "settlement": settlement(),
    }
    cards = []
    for slug, svg in diagrams.items():
        (here / f"{slug}.svg").write_text(svg.render(), encoding="utf-8")
        cards.append(
            f'<section><h2>{html.escape(svg.title)}</h2>'
            f'<div class="frame">{svg.render()}</div></section>'
        )
        print(f"wrote {slug}.svg ({svg.width}x{svg.height})")

    (here / "index.html").write_text(
        f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<title>Kindly — diagrams</title>
<style>
  body {{ margin: 0; padding: 48px 32px 80px; background: {PAPER}; color: {INK};
         font-family: {SANS}; }}
  h1 {{ font-family: {SERIF}; font-weight: 400; font-size: 34px; margin: 0 0 6px; }}
  p.lede {{ color: {MUTED}; max-width: 70ch; margin: 0 0 40px; }}
  section {{ margin-bottom: 56px; }}
  h2 {{ font-family: {SERIF}; font-weight: 400; font-size: 22px; margin: 0 0 14px; }}
  .frame {{ background: {PAPER}; border: 1px solid {RULE}; padding: 8px; overflow-x: auto; }}
  .frame svg {{ display: block; min-width: 1000px; }}
</style></head>
<body>
  <h1>Kindly — diagrams</h1>
  <p class="lede">Generated by <code>docs/diagrams/build.py</code>. Each diagram answers one question:
  what runs where, who decides what the agent may do, and how an agreement becomes money.</p>
  {''.join(cards)}
</body></html>
""",
        encoding="utf-8",
    )
    print("wrote index.html")


if __name__ == "__main__":
    main()
