import react from "@vitejs/plugin-react";
import { createRequire } from "node:module";
import path from "node:path";
import { defineConfig, type Plugin } from "vite";

const require = createRequire(import.meta.url);
const wcPkg = "@walletconnect/ethereum-provider";
const hubApiProxyTarget =
  process.env.VITE_HUB_API_PROXY_TARGET ?? "http://127.0.0.1:8095";

function walletConnectOptional(): Plugin {
  let resolved = false;
  try {
    require.resolve(wcPkg);
    resolved = true;
  } catch {
    resolved = false;
  }
  const stub = path.resolve(__dirname, "src/wallet/walletConnectStub.ts");
  return {
    name: "walletconnect-optional",
    resolveId(source) {
      if (source === wcPkg && !resolved) {
        return stub;
      }
      return null;
    },
  };
}

export default defineConfig({
  plugins: [react(), walletConnectOptional()],
  resolve: {
    alias: {
      "@hub-content": path.resolve(__dirname, "../hub/content"),
    },
  },
  server: {
    port: 8094,
    host: "0.0.0.0",
    fs: {
      allow: [path.resolve(__dirname, "..")],
    },
    proxy: {
      "/api": {
        target: hubApiProxyTarget,
        changeOrigin: true,
      },
    },
  },
  preview: {
    port: 8094,
    host: "0.0.0.0",
  },
});
