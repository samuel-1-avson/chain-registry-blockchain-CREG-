import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'

// https://vitejs.dev/config/
export default defineConfig({
  cacheDir: process.env.VITE_CACHE_DIR || 'node_modules/.vite',
  plugins: [
    react(),
    VitePWA({
      registerType: 'autoUpdate',
      injectRegister: 'auto',
      includeAssets: ['pwa-icon.svg'],
      manifest: {
        name: 'Chain Registry Explorer',
        short_name: 'CRegExplorer',
        description: 'Block explorer for the Chain Registry L1 - blocks, validators, packages, bridge, governance.',
        theme_color: '#0a0b0f',
        background_color: '#0a0b0f',
        display: 'standalone',
        start_url: '/',
        scope: '/',
        icons: [
          { src: 'pwa-icon.svg', sizes: 'any', type: 'image/svg+xml', purpose: 'any maskable' },
        ],
      },
      workbox: {
        globPatterns: ['**/*.{js,css,html,svg,woff2}'],
        navigateFallback: '/index.html',
        navigateFallbackDenylist: [/^\/v1\//, /^\/api\//, /^\/rpc$/, /^\/rpc\//, /^\/api-docs/],
        // Live chain responses must never survive a reset at the same URL.
        // Keep offline caching for static assets only.
        runtimeCaching: [
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/chain\/stats$/,
            handler: 'NetworkOnly',
            options: {
              cacheName: 'api-chain-stats',
              expiration: { maxEntries: 4, maxAgeSeconds: 60 },
            },
          },
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/blocks(\?.*)?$/,
            handler: 'NetworkOnly',
            options: {
              cacheName: 'api-blocks-list',
              expiration: { maxEntries: 20, maxAgeSeconds: 300 },
            },
          },
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/blocks\/\d+$/,
            handler: 'NetworkOnly',
            options: {
              cacheName: 'api-block-detail',
              expiration: { maxEntries: 200, maxAgeSeconds: 86400 },
            },
          },
        ],
      },
    }),
  ],
  define: {
    // Tree-shake Anvil test accounts out of mainnet builds
    '__IS_TESTNET__': JSON.stringify((process.env.VITE_NETWORK || 'testnet') !== 'mainnet'),
  },
  server: {
    port: 3007,
    host: true,
    proxy: {
      '/v1/relayer': {
        target: process.env.CREG_RELAYER_URL || process.env.CREG_NODE_URL || 'https://api.testnet.cregnet.dev',
        changeOrigin: true,
        secure: true,
      },
      '/api/': {
        target: process.env.CREG_FAUCET_URL || 'https://faucet.testnet.cregnet.dev',
        changeOrigin: true,
        secure: true,
      },
      '/v1': process.env.CREG_NODE_URL || 'http://127.0.0.1:8080',
      '/api-docs': process.env.CREG_NODE_URL || 'http://127.0.0.1:8080',
      '/chain-spec.json': {
        target: 'https://spec.testnet.cregnet.dev',
        changeOrigin: true,
        secure: true,
      },
      '/phase-c': {
        target: 'https://spec.testnet.cregnet.dev',
        changeOrigin: true,
        secure: true,
      },
      '/rpc': {
        // Match the deployed explorer's real Sepolia RPC route. A local Anvil
        // endpoint is not part of the real-node explorer workflow.
        target: 'https://explorer.testnet.cregnet.dev',
        changeOrigin: true,
        secure: true,
      },
    },
  }
})
