import { useEffect, useRef } from "react";

/**
 * The door's living background, second take (owner's words: "complesso, non
 * astratto"). Not blobs: a drifting constellation -- nodes, the links between
 * the near ones, and pulses that travel the links. It is the product's own
 * shape: an organization, alive.
 *
 * Canvas, one rAF loop, node count scaled to the viewport, colors keyed to
 * the theme class and re-read when it flips. Under prefers-reduced-motion a
 * single static frame is drawn and the loop never starts.
 */
export function DoorBackground() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let raf = 0;
    let running = true;

    type Node = { x: number; y: number; vx: number; vy: number; r: number };
    type Pulse = { a: number; b: number; t: number; speed: number };
    let nodes: Node[] = [];
    let pulses: Pulse[] = [];
    const LINK_DIST = 150;

    const palette = () => {
      const dark = document.documentElement.classList.contains("dark");
      return dark
        ? { node: "rgba(157,123,255,0.9)", link: "157,123,255", pulse: "rgba(240,192,122,0.95)" }
        : { node: "rgba(124,92,255,0.75)", link: "124,92,255", pulse: "rgba(199,21,133,0.9)" };
    };
    let colors = palette();
    const themeWatch = new MutationObserver(() => {
      colors = palette();
      if (reduce) frame(0, true);
    });
    themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

    const seed = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = canvas.offsetWidth * dpr;
      canvas.height = canvas.offsetHeight * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const area = canvas.offsetWidth * canvas.offsetHeight;
      const count = Math.min(140, Math.max(50, Math.round(area / 16000)));
      nodes = Array.from({ length: count }, () => ({
        x: Math.random() * canvas.offsetWidth,
        y: Math.random() * canvas.offsetHeight,
        vx: (Math.random() - 0.5) * 0.35,
        vy: (Math.random() - 0.5) * 0.35,
        r: 1.2 + Math.random() * 1.8,
      }));
      pulses = [];
    };

    const frame = (_ts: number, once = false) => {
      const w = canvas.offsetWidth;
      const h = canvas.offsetHeight;
      ctx.clearRect(0, 0, w, h);

      if (!once) {
        for (const n of nodes) {
          n.x += n.vx;
          n.y += n.vy;
          if (n.x < -10) n.x = w + 10;
          if (n.x > w + 10) n.x = -10;
          if (n.y < -10) n.y = h + 10;
          if (n.y > h + 10) n.y = -10;
        }
      }

      // Links between near nodes, fading with distance.
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const dx = nodes[i].x - nodes[j].x;
          const dy = nodes[i].y - nodes[j].y;
          const d2 = dx * dx + dy * dy;
          if (d2 < LINK_DIST * LINK_DIST) {
            const alpha = 0.28 * (1 - Math.sqrt(d2) / LINK_DIST);
            ctx.strokeStyle = `rgba(${colors.link},${alpha.toFixed(3)})`;
            ctx.lineWidth = 1;
            ctx.beginPath();
            ctx.moveTo(nodes[i].x, nodes[i].y);
            ctx.lineTo(nodes[j].x, nodes[j].y);
            ctx.stroke();
          }
        }
      }

      ctx.fillStyle = colors.node;
      for (const n of nodes) {
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
        ctx.fill();
      }

      if (!once) {
        // A few pulses at a time, travelling a live link each.
        if (pulses.length < 6 && Math.random() < 0.05) {
          const a = Math.floor(Math.random() * nodes.length);
          let best = -1;
          let bestD = LINK_DIST * LINK_DIST;
          for (let j = 0; j < nodes.length; j++) {
            if (j === a) continue;
            const dx = nodes[a].x - nodes[j].x;
            const dy = nodes[a].y - nodes[j].y;
            const d2 = dx * dx + dy * dy;
            if (d2 < bestD) {
              bestD = d2;
              best = j;
            }
          }
          if (best >= 0) pulses.push({ a, b: best, t: 0, speed: 0.012 + Math.random() * 0.015 });
        }
        pulses = pulses.filter((p) => p.t <= 1);
        for (const p of pulses) {
          p.t += p.speed;
          const ax = nodes[p.a].x;
          const ay = nodes[p.a].y;
          const x = ax + (nodes[p.b].x - ax) * p.t;
          const y = ay + (nodes[p.b].y - ay) * p.t;
          ctx.fillStyle = colors.pulse;
          ctx.beginPath();
          ctx.arc(x, y, 2.4, 0, Math.PI * 2);
          ctx.fill();
        }
        if (running) raf = requestAnimationFrame(frame);
      }
    };

    seed();
    if (reduce) {
      frame(0, true);
    } else {
      raf = requestAnimationFrame(frame);
    }
    const onResize = () => {
      seed();
      if (reduce) frame(0, true);
    };
    window.addEventListener("resize", onResize);
    return () => {
      running = false;
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", onResize);
      themeWatch.disconnect();
    };
  }, []);

  return (
    <canvas
      ref={ref}
      aria-hidden
      className="pointer-events-none fixed inset-0 h-full w-full"
    />
  );
}
