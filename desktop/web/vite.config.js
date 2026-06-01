import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { fileURLToPath, URL } from "node:url";
import * as vueCompiler from "vue/compiler-sfc";

export default defineConfig({
  plugins: [
    vue({
      // Work around flaky auto-resolution in some pnpm/node setups.
      compiler: vueCompiler
    })
  ],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url))
    }
  }
});
