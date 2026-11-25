import { useEffect, useRef, useState } from 'react'
import { API_BASE, NODE_ROUTE_MODE, isObserverUnavailable, nodeFetch, ApiError } from '../api/node.js'

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
 * Subscribe to the node event stream with exponential reconnect.
 * onEvent receives the parsed JSON payload (or raw string if not JSON).
 * Returns { state, lastEventAt, reconnectAttempt }.
 */
const DEFAULT_EVENTS_PATH = NODE_ROUTE_MODE === 'legacy' ? '/v1/events' : '/v1/index/events'
const REGISTRY_EVENTS = [
  // SSE v2 events (registry_view::SseEventKindV2)
  'package_verified',
  'package_quarantined',
  'quarantine_resolved',
  'quarantine_expired',
  'package_revoked_v2',
  'witness_committed',
  'bounty_awarded',
  'block_committed',
  // Legacy v1 events
  'package_submitted',
  'package_rejected',
  'package_revoked',
  'block_produced',
  'validator_voted',
]

export function useSse({ path, onEvent, eventTypes = REGISTRY_EVENTS, enabled = true } = {}) {
  const [state, setState] = useState(enabled ? SSE_STATE.Connecting : SSE_STATE.Idle)
  const [lastEventAt, setLastEventAt] = useState(null)
  const [reconnectAttempt, setReconnectAttempt] = useState(0)
  const [lastEventId, setLastEventId] = useState(null)
  const lastEventIdRef = useRef(null)
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
    let lastActivityAt = 0
    let resolvedPath = path || DEFAULT_EVENTS_PATH

    const handlePayload = (raw, type, evtId) => {
      setLastEventAt(Date.now())
      lastActivityAt = Date.now()
      setState(SSE_STATE.Live)
      let parsed
      try { parsed = JSON.parse(raw) } catch { parsed = raw }
      const envelopeId = (evtId && String(evtId).trim())
        || (parsed && typeof parsed === 'object' && typeof parsed.id === 'string' ? parsed.id : null)
      if (envelopeId) {
        lastEventIdRef.current = envelopeId
        setLastEventId(envelopeId)
      }
      onEventRef.current?.(parsed, type, envelopeId)
    }

    const resolveEventsPath = async () => {
      if (path || NODE_ROUTE_MODE !== 'auto') return path || DEFAULT_EVENTS_PATH
      try {
        await nodeFetch('/v1/index/quarantine-queue?limit=1')
        return '/v1/index/events'
      } catch {
        // Indexer not mounted; check v4 node routes next.
      }
      try {
        await nodeFetch('/v1/public/v4/quarantine?limit=1')
        return '/v1/public/v4/events'
      } catch {
        // Fall through to v1 public health check.
      }
      try {
        await nodeFetch('/v1/public/health')
        return '/v1/public/events'
      } catch (error) {
        if (error instanceof ApiError && [404, 405, 501].includes(error.status)) {
          return '/v1/events'
        }
        return '/v1/public/events'
      }
    }

    const connect = async () => {
      if (cancelled) return
      // The edge explicitly reports observer_unavailable during bootstrap.
      // Wait for the shared API backoff window instead of opening an SSE
      // connection that can only return another 503.
      if (isObserverUnavailable()) {
        setState(SSE_STATE.Error)
        attempt += 1
        setReconnectAttempt(attempt)
        reconnectTimer = setTimeout(connect, 30_000)
        return
      }
      if (attempt === 0) {
        resolvedPath = await resolveEventsPath()
      }
      if (cancelled) return
      const resumeSuffix = lastEventIdRef.current
        ? `${resolvedPath.includes('?') ? '&' : '?'}after=${encodeURIComponent(lastEventIdRef.current)}`
        : ''
      const url = joinUrl(API_BASE, `${resolvedPath}${resumeSuffix}`)
      setState(SSE_STATE.Connecting)

      es = new EventSource(url)
      es.onopen = () => {
        if (cancelled) return
        attempt = 0
        setReconnectAttempt(0)
        setState(SSE_STATE.Live)
        setLastEventAt(Date.now())
        lastActivityAt = Date.now()
      }
      es.onmessage = (evt) => handlePayload(evt.data, 'message', evt.lastEventId)
      if (Array.isArray(eventTypes)) {
        for (const t of eventTypes) {
          es.addEventListener(t, (evt) => handlePayload(evt.data, t, evt.lastEventId))
        }
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
        if (!lastActivityAt) return prev
        if (Date.now() - lastActivityAt > 30_000) return SSE_STATE.Stale
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

  return { state, lastEventAt, reconnectAttempt, lastEventId }
}
