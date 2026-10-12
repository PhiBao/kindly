import type { NextConfig } from "next";

/**
 * The web app talks to the Rust server. By default it proxies /api and /c
 * through Next so the browser sees a single origin (no CORS, cookies work).
 * Set KINDLY_API_ORIGIN to point at a different server.
 */
const apiOrigin = process.env.KINDLY_API_ORIGIN ?? "http://localhost:8787";

const nextConfig: NextConfig = {
  async rewrites() {
    return [
      { source: "/api/:path*", destination: `${apiOrigin}/api/:path*` },
    ];
  },
  env: {
    NEXT_PUBLIC_KINDLY_API: process.env.NEXT_PUBLIC_KINDLY_API ?? "",
  },
};

export default nextConfig;
