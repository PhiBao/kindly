"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { api, subscribeEvents, type Bootstrap } from "@/lib/api-client";

/**
 * Dashboard data + live updates.
 *
 * Everything on the desk comes from /api/bootstrap; server-sent events prompt
 * a debounced refresh so the ledger moves while you watch it.
 */
export function useBootstrap() {
  const [data, setData] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [lastEvent, setLastEvent] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const next = await api.bootstrap();
      setData(next);
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    const unsubscribe = subscribeEvents((event) => {
      setLastEvent(event.kind);
      if (pending.current) clearTimeout(pending.current);
      pending.current = setTimeout(() => void refresh(), 220);
    });
    // A slow poll as a safety net for dropped streams.
    const interval = setInterval(() => void refresh(), 15000);
    return () => {
      unsubscribe();
      clearInterval(interval);
      if (pending.current) clearTimeout(pending.current);
    };
  }, [refresh]);

  return { data, error, loading, refresh, lastEvent };
}
