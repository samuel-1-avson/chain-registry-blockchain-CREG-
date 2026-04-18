import { useEffect, useRef, useState } from 'react'

/**
 * Run an async function once (per deps change) with cancellation.
 * Meant for detail pages that load a single resource.
 */
export function useFetch(fn, { deps = [], enabled = true } = {}) {
  const [data, setData] = useState(null)
  const [error, setError] = useState(null)
  const [loading, setLoading] = useState(enabled)
  const fnRef = useRef(fn)
  fnRef.current = fn
  const refetchRef = useRef(null)

  useEffect(() => {
    if (!enabled) {
      setLoading(false)
      return
    }
    let cancelled = false
    const controller = new AbortController()
    setLoading(true)
    setError(null)

    const run = async () => {
      try {
        const result = await fnRef.current(controller.signal)
        if (!cancelled) {
          setData(result)
          setError(null)
        }
      } catch (e) {
        if (!cancelled && !controller.signal.aborted) setError(e)
      } finally {
        if (!cancelled) setLoading(false)
      }
    }
    refetchRef.current = run
    run()

    return () => {
      cancelled = true
      controller.abort()
    }
  }, [enabled, ...deps])

  return { data, error, loading, refetch: () => refetchRef.current?.() }
}
