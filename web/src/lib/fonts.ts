import localFont from "next/font/local";

/**
 * Self-hosted fonts (all SIL Open Font License — see src/fonts/LICENSES.md).
 * Self-hosting keeps builds working offline and drops three third-party
 * requests from every page load.
 */

export const instrumentSerif = localFont({
  src: [
    { path: "../fonts/InstrumentSerif-Regular.woff2", weight: "400", style: "normal" },
    { path: "../fonts/InstrumentSerif-Italic.woff2", weight: "400", style: "italic" },
  ],
  variable: "--font-instrument-serif",
  display: "swap",
});

export const interTight = localFont({
  src: [{ path: "../fonts/InterTight-Variable.woff2", weight: "300 700", style: "normal" }],
  variable: "--font-inter-tight",
  display: "swap",
});

export const plexMono = localFont({
  src: [
    { path: "../fonts/IBMPlexMono-Regular.woff2", weight: "400", style: "normal" },
    { path: "../fonts/IBMPlexMono-Medium.woff2", weight: "500", style: "normal" },
  ],
  variable: "--font-plex-mono",
  display: "swap",
});
