import { useEffect, useRef } from 'react'
import { nodeApi } from '../api/node.js'
import { usePolling } from './usePolling.js'

const DEGRADED_POLL_MS = 30_000

// SSE-first chain stats (ADR-019 ECA-37): triggers a refresh when SSE commits
// a block while live, and only polls at >= 30s in degraded mode.
export function useChainStats(optionsOrInterval = {}) {
  const opts = optionsOrInterval && typeof optionsOrInterval === 'object' ? optionsOrInterval : {}
  const { sseState = 'connecting', sseTick = 0 } = opts
  const degraded = sseState !== 'live'
  const query = usePolling((signal) => nodeApi.chainStats(signal), {
    intervalMs: DEGRADED_POLL_MS,
    enabled: degraded,
  })
  const initialFetchedRef = useRef(false)
  const refetchRef = useRef(query.refetch)
  refetchRef.current = query.refetch

  useEffect(() => {
    if (!initialFetchedRef.current) {
      initialFetchedRef.current = true
      refetchRef.current?.()
    }
  }, [])

  useEffect(() => {
    if (sseTick > 0) {
      refetchRef.current?.()
    }
  }, [sseTick])

  return query
}

export function useRuntimeConfig() {
  return usePolling((signal) => nodeApi.runtimeConfig(signal), { intervalMs: 60_000 })
}
