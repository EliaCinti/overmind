import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// The Rust server owns the API and the live-update socket. In dev, Vite serves
// the UI and proxies those to the backend so the frontend always talks to the
// same origin it will in production (where the server serves the built SPA).
const BACKEND = process.env.OVERMIND_BACKEND ?? "http://127.0.0.1:7070";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    proxy: {
      "/api": { target: BACKEND, changeOrigin: true },
      "/ws": { target: BACKEND, ws: true, changeOrigin: true },
    },
  },
  build: { outDir: "dist" },
});
