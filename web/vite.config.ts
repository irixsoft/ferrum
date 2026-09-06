import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";
import { fileURLToPath, URL } from "node:url";

const buildId = process.env.FERRUM_BUILD_ID ?? "dev";

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      registerType: "prompt",
      includeAssets: ["favicon.ico", "apple-touch-icon.png", "icon.svg"],
      manifest: {
        name: "Ferrum",
        short_name: "Ferrum",
        description: "Deploy and database management for a server you own.",
        theme_color: "#E3E6EA",
        background_color: "#E3E6EA",
        display: "standalone",
        start_url: "/",
        scope: "/",
        icons: [
          { src: "/pwa-192.png", sizes: "192x192", type: "image/png" },
          { src: "/pwa-512.png", sizes: "512x512", type: "image/png" },
          { src: "/pwa-maskable-192.png", sizes: "192x192", type: "image/png", purpose: "maskable" },
          { src: "/pwa-maskable-512.png", sizes: "512x512", type: "image/png", purpose: "maskable" },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,png,ico,woff,woff2}"],
        // A cached API response would write secrets to disk. Never cache the API.
        navigateFallbackDenylist: [/^\/api/, /^\/mcp/],
        runtimeCaching: [
          { urlPattern: /^\/api\//, handler: "NetworkOnly" },
          { urlPattern: /^\/mcp/, handler: "NetworkOnly" },
        ],
      },
      devOptions: { enabled: false },
    }),
  ],
  resolve: {
    alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
  },
  define: {
    __FERRUM_BUILD_ID__: JSON.stringify(buildId),
  },
  server: {
    port: 5173,
    proxy: { "/api": "http://127.0.0.1:8443" },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
