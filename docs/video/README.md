# The demo video — source

`docs/kindly-demo.mp4` is a 2:16, 1080p30, narrated and subtitled product demo. It was rendered
with [open-video-demo](https://github.com/PhiBao/open-video-demo), which records real Playwright
sessions against the deployed app and assembles them with edge-tts narration and ffmpeg.

Everything needed to re-render it is in this folder:

| File | What it is |
| --- | --- |
| `story.json` | the narration and the scripted browser pass for each scene |
| `video.json` | voice, palette, and the card copy |
| `cards/title-left.html`, `cards/close-left.html` | the paper-and-ink cards, matching the product |

## Re-rendering

```bash
git clone https://github.com/PhiBao/open-video-demo
cd open-video-demo
./scripts/setup.sh
./scripts/new.sh kindly
cp -r /path/to/kindly/docs/video/* videos/kindly/     # story, config, cards
# story.json points at live session and portal tokens that expire — set up the demo state
# first (POST /api/demo/reset, then the agent-to-agent scenario) and paste the fresh ids.
./scripts/run.sh --project kindly
```

The recorder walks the real product: it scrolls the desk, opens the negotiation room, clicks
**Their finance lead approves**, reads the invoice desk, types into the client portal, and shows
the settled invoice and the verified PayPal deliveries. Nothing is re-enacted in a video editor.

## Notes on the take

- The book in the video has **$8,350 outstanding and $900 collected**: invoice 1051 was paid
  through PayPal's sandbox with a test card before recording, so the capture and its
  `PAYMENT.CAPTURE.COMPLETED` delivery are real.
- Invoice 1042's negotiation is genuine: the offer, the counter, the policy refusal and the
  approval all ran through the product. The plan invoices it issues are real PayPal invoice
  objects.
- The client-message beat was recorded on invoice 1064 so the thread starts clean.
- The app was reset to the four-invoice story after filming; the video is a record of a real
  session, not the current demo state.
