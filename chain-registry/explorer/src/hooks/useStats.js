import { nodeApi } from '../api/node.js'
import { usePolling } from './usePolling.js'

// Polls /v1/chain/stats every 5s. Safe to call from multiple places —
// each hook instance keeps its own state but the server response is tiny.
export function useChainStats(intervalMs = 5000) {
  return usePolling((signal) => nodeApi.chainStats(signal), { intervalMs })
}

export function useRuntimeConfig() {
  return usePolling((signal) => nodeApi.runtimeConfig(signal), { intervalMs: 60_000 })
}
