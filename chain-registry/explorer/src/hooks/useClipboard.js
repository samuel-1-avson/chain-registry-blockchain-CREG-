import { useCallback, useState } from 'react'

export function useClipboard(resetMs = 1500) {
  const [copied, setCopied] = useState(false)

  const copy = useCallback(async (value) => {
    if (value == null) return false
    try {
      await navigator.clipboard.writeText(String(value))
      setCopied(true)
      setTimeout(() => setCopied(false), resetMs)
      return true
    } catch {
      // Clipboard API blocked (e.g. non-HTTPS); fall back to legacy.
      const ta = document.createElement('textarea')
      ta.value = String(value)
      ta.style.position = 'fixed'
      ta.style.opacity = '0'
      document.body.appendChild(ta)
      ta.select()
      let ok = false
      try { ok = document.execCommand('copy') } catch { ok = false }
      document.body.removeChild(ta)
      if (ok) {
        setCopied(true)
        setTimeout(() => setCopied(false), resetMs)
      }
      return ok
    }
  }, [resetMs])

  return { copied, copy }
}
