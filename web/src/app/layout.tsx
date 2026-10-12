import type { Metadata, Viewport } from "next";
import { interTight, instrumentSerif, plexMono } from "@/lib/fonts";
import "./globals.css";

export const metadata: Metadata = {
  title: "Kindly — the asking, handled",
  description:
    "Kindly is the collections agent for freelancers and small studios: it chases, negotiates and settles unpaid invoices over PayPal — so you get paid without the awkward part.",
  applicationName: "Kindly",
};

export const viewport: Viewport = {
  themeColor: "#f7f2e8",
  width: "device-width",
  initialScale: 1,
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body
        className={`${instrumentSerif.variable} ${interTight.variable} ${plexMono.variable} min-h-screen antialiased`}
      >
        {children}
      </body>
    </html>
  );
}
