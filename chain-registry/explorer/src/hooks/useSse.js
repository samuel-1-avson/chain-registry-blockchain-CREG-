import { useEffect, useRef, useState } from 'react'
import { API_BASE } from '../api/node.js'

// Connection lifecycle states surfaced to consumers so the UI can render
// a "connecting / live / stale / offline" banner instead of silently polling.
export const SSE_STATE = Object.freeze({
  Idle: 'idle',
  Connecting: 'connecting',
  Live: 'live',
  Stale: 'stale',
  Error: 'error',
})

const joinUrl = (base, path) => {
  if (!path.startsWith('/')) path = `/${path}`
  if (!base) return path
  return `${base.replace(/\/$/, '')}${path}`
}

/**
 * Subscribe to /v1/events (or a custom SSE endpoint) with exponential reconnect.
 * onEvent receives the parsed JSON payload (or raw string if not JSON).
 * Returns { state, lastEventAt, reconnectAttempt }.
 */
export function useSse({ path = '/v1/events', onEvent, eventTypes = null, enabled = true } = {}) {
  const [state, setState] = useState(enabled ? SSE_STATE.Connecting : SSE_STATE.Idle)
  const [lastEventAt, setLastEventAt] = useState(null)
  const [reconnectAttempt, setReconnectAttempt] = useState(0)
  const onEventRef = useRef(onEvent)
  onEventRef.current = onEvent

  useEffect(() => {
    if (!enabled) {
      setState(SSE_STATE.Idle)
      return
    }
    let cancelled = false
    let es = null
    let attempt = 0
    let reconnectTimer = null
    let staleTimer = null

    const handlePayload = (raw, type) => {
      setLastEventAt(Date.now())
      setState(SSE_STATE.Live)
      let parsed
      try { parsed = JSON.parse(raw) } catch { parsed = raw }
      onEventRef.current?.(parsed, type)
    }

    const connect = () => {
      if (cancelled) return
      const url = joinUrl(API_BASE, path)
      setState(SSE_STATE.Connecting)
      es = new EventSource(url)
      es.onopen = () => {
        if (cancelled) return
        attempt = 0
        setReconnectAttempt(0)
        setState(SSE_STATE.Live)
        setLastEventAt(Date.now())
      }
      es.onmessage = (evt) => handlePayload(evt.data, 'message')
      if (Array.isArray(eventTypes)) {
        for (const t of eventTypes) es.addEventListener(t, (evt) => handlePayload(evt.data, t))
      }
      es.onerror = () => {
        if (cancelled) return
        es?.close()
        es = null
        setState(SSE_STATE.Error)
        attempt += 1
        setReconnectAttempt(attempt)
        const delay = Math.min(30_000, 500 * 2 ** Math.min(attempt, 6))
        reconnectTimer = setTimeout(connect, delay)
      }
    }

    connect()

    staleTimer = setInterval(() => {
      setState((prev) => {
        if (prev !== SSE_STATE.Live) return prev
        if (!lastEventAt) return prev
        if (Date.now() - lastEventAt > 30_000) return SSE_STATE.Stale
        return prev
      })
    }, 5_000)

    return () => {
      cancelled = true
      clearTimeout(reconnectTimer)
      clearInterval(staleTimer)
      es?.close()
    }
  }, [enabled, path, eventTypes?.join('|')])

  return { state, lastEventAt, reconnectAttempt }
}
