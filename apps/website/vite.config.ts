import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// A plain static marketing SPA. No Tauri server pinning (unlike apps/desktop);
// just React + an es2022 build into `dist`, which Vercel serves as-is.
export default defineConfig({
  plugins: [react()],
  build: {
    target: "es2022",
    outDir: "dist",
  },
});
