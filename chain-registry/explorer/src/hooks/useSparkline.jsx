import { useCallback, useRef, useState } from 'react'

/**
 * Accumulate a rolling buffer of numeric samples for sparkline SVGs.
 *
 * Usage:
 *   const spark = useSparkline({ maxSamples: 30 })
 *   // on new data:
 *   spark.push(newValue)
 *   // in render:
 *   <Sparkline data={spark.data} width={120} height={32} />
 */
export function useSparkline({ maxSamples = 30 } = {}) {
  const bufRef = useRef([])
  const [data, setData] = useState([])

  const push = useCallback((value) => {
    const num = Number(value)
    if (!Number.isFinite(num)) return
    const buf = bufRef.current
    buf.push(num)
    if (buf.length > maxSamples) buf.shift()
    setData([...buf])
  }, [maxSamples])

  const reset = useCallback(() => {
    bufRef.current = []
    setData([])
  }, [])

  return { data, push, reset }
}

/**
 * Inline SVG sparkline. Renders a polyline from numeric data.
 * Designed for small inline usage in stat cards.
 */
export function Sparkline({
  data = [],
  width = 120,
  height = 32,
  color = 'var(--accent-primary-light)',
  fillOpacity = 0.08,
  strokeWidth = 1.5,
  style = {},
}) {
  if (data.length < 2) {
    return (
      <svg width={width} height={height} style={{ display: 'block', ...style }}>
        <line
          x1={0} y1={height / 2} x2={width} y2={height / 2}
          stroke="var(--border)" strokeWidth={1} strokeDasharray="3,3"
        />
      </svg>
    )
  }

  const min = Math.min(...data)
  const max = Math.max(...data)
  const range = max - min || 1
  const pad = 2

  const points = data.map((v, i) => {
    const x = (i / (data.length - 1)) * width
    const y = height - pad - ((v - min) / range) * (height - pad * 2)
    return `${x.toFixed(1)},${y.toFixed(1)}`
  })

  const polyline = points.join(' ')
  const fillPath = `M0,${height} ${points.map((p, i) => (i === 0 ? `L${p}` : `L${p}`)).join(' ')} L${width},${height} Z`

  return (
    <svg width={width} height={height} style={{ display: 'block', ...style }} aria-hidden="true">
      <path d={fillPath} fill={color} opacity={fillOpacity} />
      <polyline
        points={polyline}
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* Dot on the last point */}
      {data.length > 0 && (() => {
        const lastX = width
        const lastY = height - pad - ((data[data.length - 1] - min) / range) * (height - pad * 2)
        return <circle cx={lastX} cy={lastY} r={2.5} fill={color} />
      })()}
    </svg>
  )
}
