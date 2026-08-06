import { useEffect, useRef, useState } from "react";
import type { Notification } from "./api";

/**
 * Subscribe to the server's live-update socket.
 *
 * Two kinds of frame. `{ type: "changed", company_id }` (and `hello` on
 * connect / resync) is coarse by design: we just bump a counter the caller
 * uses to refetch, so the client is impossible to desync. `{ type:
 * "notification" }` carries content — something the company wants to tell you
 * right now (ADR-0020) — and is handed over as-is for a toast. That
 * notification is already stored server-side; the frame is only the fast path,
 * never the record.
 *
 * Auto-reconnects with backoff.
 */
export function useLive(
  onChange: (companyId: string | null) => void,
  onNotification?: (companyId: string | null, notification: Notification) => void,
) {
  const [connected, setConnected] = useState(false);
  const cbRef = useRef(onChange);
  cbRef.current = onChange;
  const noteRef = useRef(onNotification);
  noteRef.current = onNotification;

  useEffect(() => {
    let socket: WebSocket | null = null;
    let closed = false;
    let retry = 500;

    const connect = () => {
      const proto = location.protocol === "https:" ? "wss" : "ws";
      socket = new WebSocket(`${proto}://${location.host}/ws`);
      socket.onopen = () => {
        setConnected(true);
        retry = 500;
      };
      socket.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data);
          if (msg.type === "hello") cbRef.current(null);
          else if (msg.type === "changed") cbRef.current(msg.company_id ?? null);
          else if (msg.type === "notification" && msg.notification) {
            noteRef.current?.(msg.company_id ?? null, msg.notification as Notification);
          }
        } catch {
          // ignore malformed frames
        }
      };
      socket.onclose = () => {
        setConnected(false);
        if (closed) return;
        setTimeout(connect, retry);
        retry = Math.min(retry * 2, 8000);
      };
      socket.onerror = () => socket?.close();
    };
    connect();

    return () => {
      closed = true;
      socket?.close();
    };
  }, []);

  return { connected };
}
