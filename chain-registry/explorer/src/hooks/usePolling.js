import { useEffect, useRef, useState } from 'react'

/**
 * Poll an async function at a fixed interval with cancellation + pause-on-hidden.
 * Returns { data, error, loading, refetch }.
 */
export function usePolling(fn, { intervalMs = 5000, enabled = true, deps = [] } = {}) {
  const [data, setData] = useState(null)
  const [error, setError] = useState(null)
  const [loading, setLoading] = useState(false)
  const fnRef = useRef(fn)
  fnRef.current = fn
  const refetchRef = useRef(null)

  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    const controller = new AbortController()
    let timer = null

    const run = async () => {
      setLoading(true)
      try {
        const result = await fnRef.current(controller.signal)
        if (cancelled) return
        setData(result)
        setError(null)
      } catch (e) {
        if (cancelled || controller.signal.aborted) return
        setError(e)
      } finally {
        if (!cancelled) setLoading(false)
      }
    }

    refetchRef.current = run

    const tick = async () => {
      await run()
      if (cancelled) return
      if (document.visibilityState === 'hidden') {
        timer = setTimeout(tick, Math.max(intervalMs, 15000))
      } else {
        timer = setTimeout(tick, intervalMs)
      }
    }
    tick()

    const onVisibility = () => {
      if (document.visibilityState === 'visible') {
        clearTimeout(timer)
        tick()
      }
    }
    document.addEventListener('visibilitychange', onVisibility)

    return () => {
      cancelled = true
      controller.abort()
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', onVisibility)
    }
  }, [enabled, intervalMs, ...deps])

  return { data, error, loading, refetch: () => refetchRef.current?.() }
}
