import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react()],
  define: {
    // Tree-shake Anvil test accounts out of mainnet builds
    '__IS_TESTNET__': JSON.stringify((process.env.VITE_NETWORK || 'testnet') !== 'mainnet'),
  },
  server: {
    port: 3000,
    host: true,
    proxy: {
      '/v1': 'http://127.0.0.1:8080',
      '/api': 'http://127.0.0.1:8082',
      '/rpc': {
        target: 'http://127.0.0.1:8545',
        changeOrigin: true,
        rewrite: () => '/',
      },
    },
  }
})
