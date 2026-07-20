import { useEffect, useRef, useState } from "react";

/**
 * Subscribe to the server's live-update socket. The server sends coarse
 * `{ type: "changed", company_id }` (and `hello` on connect / resync); we
 * simply bump a counter the caller uses to refetch. Auto-reconnects with
 * backoff. Coarse-by-design keeps the client impossible to desync.
 */
export function useLive(onChange: (companyId: string | null) => void) {
  const [connected, setConnected] = useState(false);
  const cbRef = useRef(onChange);
  cbRef.current = onChange;

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
