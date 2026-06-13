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
        navigateFallbackDenylist: [/^\/v1\//, /^\/rpc$/, /^\/rpc\//, /^\/api-docs/],
        runtimeCaching: [
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/chain\/stats$/,
            handler: 'StaleWhileRevalidate',
            options: {
              cacheName: 'api-chain-stats',
              expiration: { maxEntries: 4, maxAgeSeconds: 60 },
            },
          },
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/blocks(\?.*)?$/,
            handler: 'StaleWhileRevalidate',
            options: {
              cacheName: 'api-blocks-list',
              expiration: { maxEntries: 20, maxAgeSeconds: 300 },
            },
          },
          {
            urlPattern: /^https?:\/\/[^/]+\/v1\/blocks\/\d+$/,
            handler: 'CacheFirst',
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
        target: 'http://127.0.0.1:8083',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/v1\/relayer/, '/v1/relayer')
      },
      '/v1': 'http://127.0.0.1:8080',
      '/api-docs': 'http://127.0.0.1:8080',
      '/rpc': {
        target: 'http://127.0.0.1:8545',
        changeOrigin: true,
        rewrite: () => '/',
      },
    },
  }
})
