<script lang="ts">
  import type { BalancePoint, TxRecord, UtxoRecord } from '../lib/types'
  import { displayUnit, balancesHidden, nodeStatus } from '../stores/settings'

  let {
    points,
    txs = [],
    utxos = [],
  }: {
    points: BalancePoint[]
    txs?: TxRecord[]
    utxos?: UtxoRecord[]
  } = $props()

  // ── View & range state ────────────────────────────────────────────────────
  type View        = 'balance' | 'cashflow' | 'activity'
  type Range       = '3m' | '1y' | '2y' | 'all'
  type Granularity = 'week' | 'month' | 'quarter'

  let view  = $state<View>('balance')
  let range = $state<Range>('all')

  // ── Date range filter ─────────────────────────────────────────────────────
  let filteredData = $derived.by(() => {
    if (range === 'all' || points.length === 0) return { pts: points, hasAnchor: false }
    const days = ({ '3m': 90, '1y': 365, '2y': 730 } as Record<string, number>)[range]
    const latest = points[points.length - 1].timestamp
      ? new Date(points[points.length - 1].timestamp!).getTime()
      : Date.now()
    const cutoff = latest - days * 86_400_000
    let anchorIdx = -1
    const inRange: BalancePoint[] = []
    for (let i = 0; i < points.length; i++) {
      const ts = points[i].timestamp ? new Date(points[i].timestamp!).getTime() : null
      if (ts !== null && ts < cutoff) anchorIdx = i
      else inRange.push(points[i])
    }
    return anchorIdx >= 0
      ? { pts: [points[anchorIdx], ...inRange], hasAnchor: true }
      : { pts: inRange, hasAnchor: false }
  })

  let filteredPoints = $derived(filteredData.pts)

  // ── Granularity ──────────────────────────────────────────────────────────
  // 'auto' picks based on range; week/month/quarter override.
  type GranularitySel = 'auto' | Granularity
  let granularitySel = $state<GranularitySel>('auto')

  let autoGranularity = $derived.by((): Granularity => {
    if (range === '3m') return 'week'
    if (range === '1y' || range === '2y') return 'month'
    if (points.length < 2) return 'month'
    const first = points[0].timestamp ? new Date(points[0].timestamp).getTime() : null
    const last  = points[points.length - 1].timestamp
      ? new Date(points[points.length - 1].timestamp!).getTime() : null
    if (!first || !last) return 'month'
    return (last - first) / (365.25 * 86_400_000) > 2 ? 'quarter' : 'month'
  })
  let granularity = $derived(granularitySel === 'auto' ? autoGranularity : granularitySel)

  const GRAN_LABEL: Record<Granularity, string> = {
    week:    'Weekly',
    month:   'Monthly',
    quarter: 'Quarterly',
  }

  // ── Summary stats ─────────────────────────────────────────────────────────
  /// Full number under 10M; abbreviated above. Avoids "525.0k" hiding 15 sats.
  function formatSummary(sats: number, signed: boolean = false): string {
    if ($displayUnit === 'btc') {
      const btc = sats / 1e8
      const sign = signed && sats > 0 ? '+' : ''
      return sign + (Math.abs(btc) >= 0.001 ? btc.toFixed(4) : btc.toFixed(8)) + ' BTC'
    }
    const abs = Math.abs(sats)
    const sign = signed && sats > 0 ? '+' : signed && sats < 0 ? '−' : ''
    const mag  = signed ? abs : sats
    if (abs >= 10_000_000) return sign + (Math.abs(mag) / 1_000_000).toFixed(2) + 'M sats'
    return sign + mag.toLocaleString() + ' sats'
  }

  let summary = $derived.by(() => {
    const { pts, hasAnchor } = filteredData
    let received = 0, sent = 0
    const start = hasAnchor ? 1 : 0
    for (let i = start; i < pts.length; i++) {
      const delta = pts[i].balance_sats - (i > 0 ? pts[i - 1].balance_sats : 0)
      if (delta > 0) received += delta
      else sent += Math.abs(delta)
    }
    return { received, sent, net: received - sent }
  })

  // ── Chart geometry ────────────────────────────────────────────────────────
  const W   = 800
  const H   = 300
  const PAD = { top: 20, right: 24, bottom: 44, left: 56 }
  const CW  = W - PAD.left - PAD.right
  const CH  = H - PAD.top - PAD.bottom

  /// Tick values snapped to 1/2/5 × 10ⁿ.
  function niceTicks(min: number, max: number, targetCount: number = 4): number[] {
    if (max <= min) return [min]
    const rough = (max - min) / (targetCount - 1)
    const mag   = Math.pow(10, Math.floor(Math.log10(rough)))
    const norm  = rough / mag
    // Snap to the nearest "nice" multiplier in {1, 2, 5, 10}.
    let step: number
    if      (norm < 1.5)  step = 1  * mag
    else if (norm < 3.5)  step = 2  * mag
    else if (norm < 7.5)  step = 5  * mag
    else                  step = 10 * mag
    const niceMin = Math.floor(min / step) * step
    const niceMax = Math.ceil(max / step) * step
    const ticks: number[] = []
    for (let v = niceMin; v <= niceMax + 1e-6; v += step) ticks.push(v)
    return ticks
  }

  function trimZeros(s: string): string {
    return s.includes('.') ? s.replace(/\.?0+$/, '') : s
  }

  function formatBal(sats: number): string {
    if ($displayUnit === 'btc') {
      const btc = sats / 1e8
      return (Math.abs(btc) >= 0.001 ? btc.toFixed(4) : btc.toFixed(8)) + ' BTC'
    }
    const abs = Math.abs(sats)
    if (abs >= 1_000_000) return trimZeros((sats / 1_000_000).toFixed(2)) + 'M'
    if (abs >= 1_000)     return trimZeros((sats / 1_000).toFixed(1)) + 'k'
    return sats.toString()
  }

  function formatDate(ts: string | null, height: number): string {
    if (ts) return new Date(ts).toLocaleDateString(undefined, { month: 'short', year: 'numeric' })
    return '#' + height.toLocaleString()
  }

  // ── Balance chart ─────────────────────────────────────────────────────────
  let hovered = $state<{ x: number; y: number; label: string; received: boolean } | null>(null)

  let balanceChart = $derived.by(() => {
    const pts = filteredPoints
    if (pts.length < 2) return null

    const minH   = pts[0].block_height
    const lastTxH = pts[pts.length - 1].block_height

    // Extend x-axis to the node tip for ranged views.
    const tipH     = range !== 'all' ? ($nodeStatus?.tip_height ?? null) : null
    const plotMaxH = (tipH && tipH > lastTxH) ? tipH : lastTxH

    const maxB  = pts.reduce((m, p) => Math.max(m, p.balance_sats), 1)
    const minB  = pts.reduce((m, p) => Math.min(m, p.balance_sats), 0)
    const rangeB = maxB - minB || 1

    const xOf = (h: number) =>
      PAD.left + (plotMaxH === minH ? CW / 2 : ((h - minH) / (plotMaxH - minH)) * CW)
    const yOf = (b: number) =>
      PAD.top + CH - ((b - minB) / rangeB) * CH
    const y0    = yOf(0)
    const lastX = xOf(lastTxH)
    const lastY = yOf(pts[pts.length - 1].balance_sats)

    // Fill path — ends at lastX, not forced to CW
    let d = `M ${PAD.left},${y0}`
    for (let i = 0; i < pts.length; i++) {
      const x = xOf(pts[i].block_height), y = yOf(pts[i].balance_sats)
      d += i === 0
        ? ` L ${x},${y0} L ${x},${y}`
        : ` L ${x},${yOf(pts[i - 1].balance_sats)} L ${x},${y}`
    }
    d += ` L ${lastX},${y0} Z`

    // Stroke path — ends at last transaction
    let stroke = `M ${PAD.left},${y0}`
    for (let i = 0; i < pts.length; i++) {
      const x = xOf(pts[i].block_height), y = yOf(pts[i].balance_sats)
      stroke += i === 0
        ? ` L ${x},${y0} L ${x},${y}`
        : ` L ${x},${yOf(pts[i - 1].balance_sats)} L ${x},${y}`
    }
    // leave the stroke ending at lastX,lastY — no extension to right edge

    const yTicks = niceTicks(minB, maxB, 4)
      .filter(v => v >= minB - rangeB * 0.05 && v <= maxB + rangeB * 0.05)
      .map(v => ({ y: yOf(v), label: formatBal(v) }))

    // X labels evenly spaced across the full plotted range (including tip extension)
    const targetCount = 5
    const seen = new Set<string>()
    const xLabels: { x: number; label: string; anchor: string }[] = []
    for (let i = 0; i < targetCount; i++) {
      const targetH = minH + ((plotMaxH - minH) * i) / (targetCount - 1)
      let closest = 0, closestDist = Infinity
      for (let j = 0; j < pts.length; j++) {
        const dist = Math.abs(pts[j].block_height - targetH)
        if (dist < closestDist) { closestDist = dist; closest = j }
      }
      const label = formatDate(pts[closest].timestamp, pts[closest].block_height)
      if (!seen.has(label)) {
        seen.add(label)
        xLabels.push({
          x: xOf(pts[closest].block_height), label,
          anchor: i === 0 ? 'start' : i === targetCount - 1 ? 'end' : 'middle',
        })
      }
    }

    // "Today" label at the right edge when we've extended past the last tx
    const todayLabel = (tipH && tipH > lastTxH)
      ? { x: PAD.left + CW, label: 'Today', anchor: 'end' }
      : null

    const dots = pts
      .map((p, i) => ({ x: xOf(p.block_height), y: yOf(p.balance_sats), isAnchor: i === 0 && filteredData.hasAnchor }))
      .filter(d => !d.isAnchor)

    const hitAreas = pts.map((p, i) => {
      const x     = xOf(p.block_height)
      const nextX = i < pts.length - 1 ? xOf(pts[i + 1].block_height) : lastX
      const delta = i === 0 ? null : p.balance_sats - pts[i - 1].balance_sats
      const deltaStr = delta != null
        ? `  ${delta >= 0 ? '+' : '−'}${formatBal(Math.abs(delta))}`
        : ''
      return {
        x, nextX, y: yOf(p.balance_sats),
        received:  delta != null ? delta >= 0 : true,
        label:     formatDate(p.timestamp, p.block_height) + '  ' + formatBal(p.balance_sats) + deltaStr,
        isAnchor:  i === 0 && filteredData.hasAnchor,
      }
    })

    return { d, stroke, yTicks, xLabels, todayLabel, y0, lastX, lastY, dots, hitAreas }
  })

  // ── Cash flow chart ───────────────────────────────────────────────────────
  function floorToGran(d: Date, gran: Granularity): Date {
    if (gran === 'month')   return new Date(d.getFullYear(), d.getMonth(), 1)
    if (gran === 'quarter') return new Date(d.getFullYear(), Math.floor(d.getMonth() / 3) * 3, 1)
    const day  = d.getDay()
    const diff = d.getDate() - day + (day === 0 ? -6 : 1)
    return new Date(d.getFullYear(), d.getMonth(), diff)
  }

  function nextGran(d: Date, gran: Granularity): Date {
    if (gran === 'month')   return new Date(d.getFullYear(), d.getMonth() + 1, 1)
    if (gran === 'quarter') return new Date(d.getFullYear(), d.getMonth() + 3, 1)
    return new Date(d.getFullYear(), d.getMonth(), d.getDate() + 7)
  }

  function labelGran(d: Date, gran: Granularity): string {
    if (gran === 'month')
      return d.toLocaleDateString(undefined, { month: 'short', year: 'numeric' })
    if (gran === 'quarter')
      return `Q${Math.floor(d.getMonth() / 3) + 1} ${d.getFullYear()}`
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
  }

  let hoveredBar = $state<{ label: string; received: number; sent: number } | null>(null)

  let cashFlowChart = $derived.by(() => {
    const { pts, hasAnchor } = filteredData
    if (pts.length < 2) return null

    const gran  = granularity
    const start = hasAnchor ? 1 : 0

    const tsPoints = pts.slice(start).filter(p => p.timestamp)
    if (tsPoints.length === 0) return null

    const firstDate = floorToGran(new Date(tsPoints[0].timestamp!), gran)
    const lastDate  = floorToGran(new Date(tsPoints[tsPoints.length - 1].timestamp!), gran)

    const bucketKeys: number[] = []
    const bucketMap = new Map<number, { label: string; received: number; sent: number }>()
    let cur = new Date(firstDate)
    while (cur <= lastDate) {
      const key = cur.getTime()
      bucketKeys.push(key)
      bucketMap.set(key, { label: labelGran(cur, gran), received: 0, sent: 0 })
      cur = nextGran(cur, gran)
    }

    for (let i = start; i < pts.length; i++) {
      if (!pts[i].timestamp) continue
      const prevBal = i > 0 ? pts[i - 1].balance_sats : 0
      const delta   = pts[i].balance_sats - prevBal
      const key     = floorToGran(new Date(pts[i].timestamp!), gran).getTime()
      const bucket  = bucketMap.get(key)
      if (!bucket) continue
      if (delta > 0) bucket.received += delta
      else bucket.sent += Math.abs(delta)
    }

    const data   = bucketKeys.map(k => bucketMap.get(k)!)
    const maxVal = data.reduce((m, d) => Math.max(m, d.received, d.sent), 1)

    // Scale bars to niceMax so the top tick label lands inside the SVG.
    const niceMax = niceTicks(0, maxVal, 3).filter(v => v > 0).pop() ?? maxVal
    const y0     = PAD.top + CH / 2
    const yScale = (CH / 2) / niceMax
    const step   = CW / data.length
    const bw     = Math.max(3, step * 0.65)

    const bars = data.map((d, i) => ({
      cx:       PAD.left + step * i + step / 2,
      recvY:    y0 - d.received * yScale,
      recvH:    d.received * yScale,
      sentY:    y0,
      sentH:    d.sent * yScale,
      label:    d.label,
      received: d.received,
      sent:     d.sent,
    }))

    // Signed labels so the +X / −X reading is unambiguous.
    const yTicks = [
      { y: y0 - niceMax * yScale,       label: '+' + formatBal(niceMax) },
      { y: y0 - (niceMax / 2) * yScale, label: '+' + formatBal(niceMax / 2) },
      { y: y0,                          label: '0' },
      { y: y0 + (niceMax / 2) * yScale, label: '−' + formatBal(niceMax / 2) },
      { y: y0 + niceMax * yScale,       label: '−' + formatBal(niceMax) },
    ]

    // Prefer year boundaries; fall back to evenly-spaced indices.
    const yearBoundaryIdxs: number[] = []
    let lastYear = -1
    for (let i = 0; i < bucketKeys.length; i++) {
      const y = new Date(bucketKeys[i]).getFullYear()
      if (y !== lastYear) { yearBoundaryIdxs.push(i); lastYear = y }
    }
    const useYearBoundaries = yearBoundaryIdxs.length >= 2 && yearBoundaryIdxs.length <= 6
    const labelIdxs: number[] = useYearBoundaries
      ? yearBoundaryIdxs
      : (() => {
          const tc = Math.min(5, bars.length)
          return Array.from({ length: tc }, (_, i) =>
            tc === 1 ? 0 : Math.round((i / (tc - 1)) * (bars.length - 1)))
        })()
    const xLabels = labelIdxs.map((idx, i) => ({
      x:      bars[idx].cx,
      label:  useYearBoundaries
        ? String(new Date(bucketKeys[idx]).getFullYear())
        : bars[idx].label,
      anchor: i === 0 ? 'start' : i === labelIdxs.length - 1 ? 'end' : 'middle',
    }))

    return { bars, bw, yTicks, xLabels, y0 }
  })

  // ── Activity charts: Fees + UTXO age ──────────────────────────────────────
  const ACT_W   = 400
  const ACT_H   = 220
  const ACT_PAD = { top: 18, right: 14, bottom: 32, left: 44 }
  const ACT_CW  = ACT_W - ACT_PAD.left - ACT_PAD.right
  const ACT_CH  = ACT_H - ACT_PAD.top - ACT_PAD.bottom

  let feesCutoff = $derived.by(() => {
    if (range === 'all') return 0
    const days = ({ '3m': 90, '1y': 365, '2y': 730 } as Record<string, number>)[range]
    return Date.now() - days * 86_400_000
  })

  let feesChart = $derived.by(() => {
    const txsWithFee = txs.filter(t =>
      t.fee_sats != null && t.timestamp &&
      new Date(t.timestamp).getTime() >= feesCutoff
    )
    if (txsWithFee.length === 0) return null

    const gran = granularity
    const firstDate = floorToGran(new Date(txsWithFee[txsWithFee.length - 1].timestamp!), gran)
    const lastDate  = floorToGran(new Date(txsWithFee[0].timestamp!), gran)

    const buckets = new Map<number, { label: string; fee: number; count: number }>()
    const keys: number[] = []
    let cur = new Date(firstDate)
    while (cur <= lastDate) {
      const k = cur.getTime()
      keys.push(k)
      buckets.set(k, { label: labelGran(cur, gran), fee: 0, count: 0 })
      cur = nextGran(cur, gran)
    }

    for (const t of txsWithFee) {
      const k = floorToGran(new Date(t.timestamp!), gran).getTime()
      const b = buckets.get(k)
      if (b) { b.fee += t.fee_sats!; b.count += 1 }
    }

    const data = keys.map(k => buckets.get(k)!)
    if (data.every(d => d.fee === 0)) return null

    const maxFee = data.reduce((m, d) => Math.max(m, d.fee), 1)
    const niceMax = niceTicks(0, maxFee, 3).filter(v => v > 0).pop() ?? maxFee

    const baseY = ACT_PAD.top + ACT_CH
    const yScale = ACT_CH / niceMax
    const step   = ACT_CW / data.length
    const bw     = Math.max(2, step * 0.7)

    const bars = data.map((d, i) => ({
      cx: ACT_PAD.left + step * i + step / 2,
      y:  baseY - d.fee * yScale,
      h:  d.fee * yScale,
      label: d.label,
      fee: d.fee, count: d.count,
    }))

    const yTicks = [
      { y: baseY,                    label: '0' },
      { y: baseY - (niceMax / 2) * yScale, label: formatBal(niceMax / 2) },
      { y: baseY - niceMax * yScale, label: formatBal(niceMax) },
    ]

    const yearBoundaryIdxs: number[] = []
    let lastYear = -1
    for (let i = 0; i < keys.length; i++) {
      const y = new Date(keys[i]).getFullYear()
      if (y !== lastYear) { yearBoundaryIdxs.push(i); lastYear = y }
    }
    const useYearBoundaries = yearBoundaryIdxs.length >= 2 && yearBoundaryIdxs.length <= 6
    const labelIdxs: number[] = useYearBoundaries
      ? yearBoundaryIdxs
      : (() => {
          const tc = Math.min(4, data.length)
          return Array.from({ length: tc }, (_, i) =>
            tc === 1 ? 0 : Math.round((i / (tc - 1)) * (data.length - 1)))
        })()
    const xLabels = labelIdxs.map((idx, i) => ({
      x: bars[idx].cx,
      label: useYearBoundaries
        ? String(new Date(keys[idx]).getFullYear())
        : data[idx].label,
      anchor: i === 0 ? 'start' : i === labelIdxs.length - 1 ? 'end' : 'middle',
    }))

    const totalFee  = data.reduce((s, d) => s + d.fee, 0)
    const totalTx   = data.reduce((s, d) => s + d.count, 0)
    const medianFee = totalTx > 0 ? Math.round(totalFee / totalTx) : 0

    return { bars, bw, yTicks, xLabels, baseY, totalFee, medianFee, totalTx }
  })

  // Buckets chosen for privacy/holding-period readability (~144 blocks/day).
  const AGE_BUCKETS = [
    { label: '<1m',    maxDays: 30 },
    { label: '1–3m',   maxDays: 90 },
    { label: '3–12m',  maxDays: 365 },
    { label: '1y+',    maxDays: Infinity },
  ]

  let utxoAgeChart = $derived.by(() => {
    if (utxos.length === 0) return null
    const counts = AGE_BUCKETS.map(b => ({ label: b.label, count: 0, totalSats: 0 }))
    let oldestDays = 0, newestDays = Infinity
    for (const u of utxos) {
      const days = u.confirmations / 144
      if (u.confirmations > 0) {
        oldestDays = Math.max(oldestDays, days)
        newestDays = Math.min(newestDays, days)
      }
      for (let i = 0; i < AGE_BUCKETS.length; i++) {
        if (days < AGE_BUCKETS[i].maxDays) {
          counts[i].count += 1
          counts[i].totalSats += u.amount_sats
          break
        }
      }
    }

    const maxCount = counts.reduce((m, b) => Math.max(m, b.count), 1)
    const niceMaxC = niceTicks(0, maxCount, 3).filter(v => v > 0).pop() ?? maxCount

    const baseY = ACT_PAD.top + ACT_CH
    const yScale = ACT_CH / niceMaxC
    const step   = ACT_CW / counts.length
    const bw     = Math.min(32, step * 0.55)

    const bars = counts.map((b, i) => ({
      cx:    ACT_PAD.left + step * i + step / 2,
      y:     baseY - b.count * yScale,
      h:     b.count * yScale,
      label: b.label,
      count: b.count,
      totalSats: b.totalSats,
    }))

    // Integer ticks only — counts are integers.
    const yTicks: { y: number; label: string }[] = []
    const tickStep = Math.max(1, Math.ceil(niceMaxC / 3))
    for (let v = 0; v <= niceMaxC; v += tickStep) {
      yTicks.push({ y: baseY - v * yScale, label: String(v) })
    }

    const total = utxos.length
    return {
      bars, bw, yTicks, baseY,
      total,
      oldestDays: oldestDays > 0 ? Math.round(oldestDays) : null,
      newestDays: newestDays < Infinity ? Math.round(newestDays) : null,
    }
  })

  function fmtDays(d: number): string {
    if (d < 1)   return '< 1d'
    if (d < 7)   return `${d}d`
    if (d < 30)  return `${Math.round(d / 7)}w`
    if (d < 365) return `${Math.round(d / 30)}mo`
    const y = Math.floor(d / 365)
    const m = Math.round((d - y * 365) / 30)
    return m === 0 ? `${y}y` : `${y}y ${m}mo`
  }

  let hoveredActFee = $state<{ x: number; y: number; label: string; fee: number; count: number } | null>(null)
  let hoveredActAge = $state<{ x: number; label: string; count: number; totalSats: number } | null>(null)
</script>

{#if $balancesHidden}
  <div class="hidden-placeholder">Balance hidden — click the balance to reveal</div>
{:else}
  <div class="controls">
    <div class="range-group">
      {#each (['3m', '1y', '2y', 'all'] as Range[]) as r (r)}
        <button class="ctrl-btn" class:active={range === r} aria-pressed={range === r} onclick={() => range = r}>
          {r === 'all' ? 'All' : r.toUpperCase()}
        </button>
      {/each}
      {#if view === 'cashflow' || view === 'activity'}
        <select
          class="gran-select"
          bind:value={granularitySel}
          aria-label="Bar granularity"
          title="How the data is bucketed into bars"
        >
          <option value="auto">Auto ({GRAN_LABEL[autoGranularity]})</option>
          <option value="week">Weekly</option>
          <option value="month">Monthly</option>
          <option value="quarter">Quarterly</option>
        </select>
      {/if}
    </div>
    <div class="view-group">
      <button class="ctrl-btn" class:active={view === 'balance'}  aria-pressed={view === 'balance'}  onclick={() => view = 'balance'}>Balance</button>
      <button class="ctrl-btn" class:active={view === 'cashflow'} aria-pressed={view === 'cashflow'} onclick={() => view = 'cashflow'}>Cash Flow</button>
      <button class="ctrl-btn" class:active={view === 'activity'} aria-pressed={view === 'activity'} onclick={() => view = 'activity'}>Activity</button>
    </div>
  </div>

  <!-- Balance view -->
  {#if view === 'balance'}
    {#if !balanceChart}
      <p class="empty">Not enough data in this range.</p>
    {:else}
      <div class="chart-wrap">
        <svg
          viewBox="0 0 {W} {H}"
          preserveAspectRatio="xMidYMid meet"
          class="chart"
          aria-labelledby="balance-chart-title"
          onpointermove={(e) => {
            const rect = (e.currentTarget as SVGSVGElement).getBoundingClientRect()
            const svgX = ((e.clientX - rect.left) / rect.width) * W
            const hit  = [...balanceChart!.hitAreas].reverse().find(a => svgX >= a.x && !a.isAnchor)
            hovered = hit ? { x: hit.x, y: hit.y, label: hit.label, received: hit.received } : null
          }}
          onpointerleave={() => hovered = null}
          role="img"
        >
          <title id="balance-chart-title">Balance history chart</title>
          {#each balanceChart.yTicks as tick, i (i)}
            <line x1={PAD.left} y1={tick.y} x2={PAD.left + CW} y2={tick.y}
              stroke="var(--border)" stroke-width="0.5" />
          {/each}

          <path d={balanceChart.d} fill="color-mix(in srgb, var(--accent) 8%, transparent)" />
          <path d={balanceChart.stroke} fill="none" stroke="var(--accent)" stroke-width="1.5" stroke-linejoin="round" />

          {#each balanceChart.dots as dot, i (i)}
            <circle cx={dot.x} cy={dot.y} r="2"
              fill="var(--surface-1)" stroke="var(--accent)" stroke-width="1.5" opacity="0.7" />
          {/each}

          {#each balanceChart.yTicks as tick, i (i)}
            <text x={PAD.left - 8} y={tick.y + 4} class="y-label">{tick.label}</text>
          {/each}

          <line x1={PAD.left} y1={PAD.top + CH} x2={PAD.left + CW} y2={PAD.top + CH}
            stroke="var(--border)" stroke-width="1" />

          {#each balanceChart.xLabels as lbl, i (i)}
            <text x={lbl.x} y={H - 14} class="x-label" text-anchor={lbl.anchor}>{lbl.label}</text>
          {/each}

          {#if balanceChart.todayLabel}
            <text x={balanceChart.todayLabel.x} y={H - 14} class="x-label today-label"
              text-anchor={balanceChart.todayLabel.anchor}>{balanceChart.todayLabel.label}</text>
          {/if}

          {#if hovered}
            <line x1={hovered.x} y1={PAD.top} x2={hovered.x} y2={PAD.top + CH}
              stroke="var(--accent)" stroke-width="0.75" stroke-dasharray="3 3" opacity="0.6" />
            <circle cx={hovered.x} cy={hovered.y} r="4" fill="var(--accent)" />
            {@const lx     = hovered.x > W - 180 ? hovered.x - 10 : hovered.x + 10}
            {@const anchor = hovered.x > W - 180 ? 'end' : 'start'}
            <text x={lx} y={hovered.y - 10} class="hover-label" text-anchor={anchor}
              fill={hovered.received ? '#52a875' : '#e05252'}
            >{hovered.label}</text>
          {/if}
        </svg>
      </div>
    {/if}

  <!-- Cash flow view -->
  {:else if view === 'cashflow'}
    {#if !cashFlowChart}
      <p class="empty">Not enough data in this range.</p>
    {:else}
      <div class="chart-wrap">
        <svg
          viewBox="0 0 {W} {H}"
          preserveAspectRatio="xMidYMid meet"
          class="chart"
          onpointerleave={() => hoveredBar = null}
          role="img"
          aria-label="Cash flow chart"
        >
          {#each cashFlowChart.yTicks as tick, i (i)}
            <line x1={PAD.left} y1={tick.y} x2={PAD.left + CW} y2={tick.y}
              stroke="var(--border)"
              stroke-width={tick.label === '0' ? 1 : 0.5} />
          {/each}

          {#each cashFlowChart.bars as bar, i (i)}
            {#if bar.recvH > 0}
              <rect
                x={bar.cx - cashFlowChart.bw / 2} y={bar.recvY}
                width={cashFlowChart.bw} height={bar.recvH}
                fill={hoveredBar?.label === bar.label
                  ? 'color-mix(in srgb, #52a875 55%, transparent)'
                  : 'color-mix(in srgb, #52a875 35%, transparent)'}
                stroke="#52a875" stroke-width="0.75" rx="1"
              />
            {/if}
            {#if bar.sentH > 0}
              <rect
                x={bar.cx - cashFlowChart.bw / 2} y={bar.sentY}
                width={cashFlowChart.bw} height={bar.sentH}
                fill={hoveredBar?.label === bar.label
                  ? 'color-mix(in srgb, #e05252 55%, transparent)'
                  : 'color-mix(in srgb, #e05252 35%, transparent)'}
                stroke="#e05252" stroke-width="0.75" rx="1"
              />
            {/if}
            <rect
              role="button"
              tabindex="0"
              aria-label="{bar.label}: +{formatBal(bar.received)} received, −{formatBal(bar.sent)} sent"
              x={bar.cx - cashFlowChart.bw / 2} y={PAD.top}
              width={cashFlowChart.bw} height={CH}
              fill="transparent"
              onpointerenter={() => hoveredBar = { label: bar.label, received: bar.received, sent: bar.sent }}
              onpointerdown={() => hoveredBar = { label: bar.label, received: bar.received, sent: bar.sent }}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { hoveredBar = { label: bar.label, received: bar.received, sent: bar.sent }; e.preventDefault() } }}
            />
          {/each}

          {#each cashFlowChart.yTicks as tick, i (i)}
            <text x={PAD.left - 8} y={tick.y + 4} class="y-label">{tick.label}</text>
          {/each}

          <!-- Heavier zero line — it's the chart's meaning anchor. -->
          <line x1={PAD.left} y1={cashFlowChart.y0} x2={PAD.left + CW} y2={cashFlowChart.y0}
            stroke="var(--text-muted)" stroke-width="1.25" opacity="0.6" />

          {#each cashFlowChart.xLabels as lbl, i (i)}
            <text x={lbl.x} y={H - 14} class="x-label" text-anchor={lbl.anchor}>{lbl.label}</text>
          {/each}

          {#if hoveredBar}
            {@const bar      = cashFlowChart.bars.find(b => b.label === hoveredBar!.label)}
            {#if bar}
            {@const flip     = bar.cx > W - 110}
            {@const tx       = flip ? bar.cx - 10 : bar.cx + 10}
            {@const anchor   = flip ? 'end' : 'start'}
            {@const ty       = PAD.top + 14}
            {@const rectX    = flip ? tx - 76 : tx - 6}
            <rect x={rectX} y={ty - 12} width="82" height="50"
              fill="var(--surface-1)" stroke="var(--border)" stroke-width="0.75" rx="3" opacity="0.95" />
            <text x={tx} y={ty}      class="hover-label" text-anchor={anchor} fill="#52a875">+{formatBal(hoveredBar.received)}</text>
            <text x={tx} y={ty + 16} class="hover-label" text-anchor={anchor} fill="#e05252">−{formatBal(hoveredBar.sent)}</text>
            <text x={tx} y={ty + 32} class="hover-label cf-date" text-anchor={anchor}>{hoveredBar.label}</text>
            {/if}
          {/if}
        </svg>
      </div>
    {/if}

  {:else}
    <div class="activity-grid">
      <section class="activity-card">
        <h3 class="card-title">UTXO age distribution</h3>
        {#if !utxoAgeChart}
          <p class="empty">No UTXOs.</p>
        {:else}
          <div class="chart-wrap">
            <svg
              viewBox="0 0 {ACT_W} {ACT_H}"
              preserveAspectRatio="xMidYMid meet"
              class="chart"
              onpointerleave={() => hoveredActAge = null}
              role="img"
              aria-label="UTXO age distribution"
            >
              {#each utxoAgeChart.bars as bar, i (i)}
                {#if bar.h > 0}
                  <rect
                    x={bar.cx - utxoAgeChart.bw / 2} y={bar.y}
                    width={utxoAgeChart.bw} height={bar.h}
                    fill={hoveredActAge?.label === bar.label
                      ? 'color-mix(in srgb, var(--accent) 55%, transparent)'
                      : 'color-mix(in srgb, var(--accent) 30%, transparent)'}
                    stroke="var(--accent)" stroke-width="0.5" rx="1"
                  />
                  <text x={bar.cx} y={bar.y - 5} class="bar-count" text-anchor="middle">{bar.count}</text>
                {/if}
                <rect
                  role="button" tabindex="0"
                  aria-label="{bar.count} UTXO{bar.count !== 1 ? 's' : ''} aged {bar.label}, totaling {formatBal(bar.totalSats)} sats"
                  x={bar.cx - utxoAgeChart.bw / 2} y={ACT_PAD.top}
                  width={utxoAgeChart.bw} height={ACT_CH}
                  fill="transparent"
                  onpointerenter={() => hoveredActAge = { x: bar.cx, label: bar.label, count: bar.count, totalSats: bar.totalSats }}
                  onpointerdown={() => hoveredActAge = { x: bar.cx, label: bar.label, count: bar.count, totalSats: bar.totalSats }}
                  onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { hoveredActAge = { x: bar.cx, label: bar.label, count: bar.count, totalSats: bar.totalSats }; e.preventDefault() } }}
                />
                <text x={bar.cx} y={ACT_H - 10} class="x-label" text-anchor="middle">{bar.label}</text>
              {/each}
              <line x1={ACT_PAD.left} y1={utxoAgeChart.baseY} x2={ACT_PAD.left + ACT_CW} y2={utxoAgeChart.baseY}
                stroke="var(--border)" stroke-width="1" />
              {#if hoveredActAge}
                {@const flip = hoveredActAge.x > ACT_W - 100}
                {@const tx   = flip ? hoveredActAge.x - 6 : hoveredActAge.x + 6}
                {@const anc  = flip ? 'end' : 'start'}
                <text x={tx} y={ACT_PAD.top + 12} class="hover-label" text-anchor={anc} fill="var(--accent)">{hoveredActAge.count} UTXO{hoveredActAge.count !== 1 ? 's' : ''}</text>
                <text x={tx} y={ACT_PAD.top + 26} class="hover-label cf-date" text-anchor={anc}>{formatBal(hoveredActAge.totalSats)} sats · age {hoveredActAge.label}</text>
              {/if}
            </svg>
          </div>
          <div class="card-stats">
            <div><span class="stat-label">UTXOs</span><span class="stat-value">{utxoAgeChart.total.toLocaleString()}</span></div>
            {#if utxoAgeChart.oldestDays !== null}
              <div><span class="stat-label">Oldest</span><span class="stat-value">{fmtDays(utxoAgeChart.oldestDays)}</span></div>
            {/if}
            {#if utxoAgeChart.newestDays !== null}
              <div><span class="stat-label">Newest</span><span class="stat-value">{fmtDays(utxoAgeChart.newestDays)}</span></div>
            {/if}
          </div>
        {/if}
      </section>

      <section class="activity-card">
        <h3 class="card-title">Fees paid over time</h3>
        {#if !feesChart}
          <p class="empty">No fee data in this range.</p>
        {:else}
          <div class="chart-wrap">
            <svg
              viewBox="0 0 {ACT_W} {ACT_H}"
              preserveAspectRatio="xMidYMid meet"
              class="chart"
              onpointerleave={() => hoveredActFee = null}
              role="img"
              aria-label="Fees paid over time"
            >
              {#each feesChart.yTicks as tick, i (i)}
                <line x1={ACT_PAD.left} y1={tick.y} x2={ACT_PAD.left + ACT_CW} y2={tick.y}
                  stroke="var(--border)" stroke-width="0.5" />
              {/each}
              {#each feesChart.bars as bar, i (i)}
                {#if bar.h > 0}
                  <rect
                    x={bar.cx - feesChart.bw / 2} y={bar.y}
                    width={feesChart.bw} height={bar.h}
                    fill={hoveredActFee?.label === bar.label
                      ? 'color-mix(in srgb, #e05252 55%, transparent)'
                      : 'color-mix(in srgb, #e05252 30%, transparent)'}
                    stroke="#e05252" stroke-width="0.5" rx="1"
                  />
                {/if}
                <rect
                  role="button" tabindex="0"
                  aria-label="{bar.label}: {formatBal(bar.fee)} sats in {bar.count} transaction{bar.count !== 1 ? 's' : ''}"
                  x={bar.cx - feesChart.bw / 2} y={ACT_PAD.top}
                  width={feesChart.bw} height={ACT_CH}
                  fill="transparent"
                  onpointerenter={() => hoveredActFee = { x: bar.cx, y: bar.y, label: bar.label, fee: bar.fee, count: bar.count }}
                  onpointerdown={() => hoveredActFee = { x: bar.cx, y: bar.y, label: bar.label, fee: bar.fee, count: bar.count }}
                  onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { hoveredActFee = { x: bar.cx, y: bar.y, label: bar.label, fee: bar.fee, count: bar.count }; e.preventDefault() } }}
                />
              {/each}
              {#each feesChart.yTicks as tick, i (i)}
                <text x={ACT_PAD.left - 6} y={tick.y + 3} class="y-label">{tick.label}</text>
              {/each}
              <line x1={ACT_PAD.left} y1={feesChart.baseY} x2={ACT_PAD.left + ACT_CW} y2={feesChart.baseY}
                stroke="var(--border)" stroke-width="1" />
              {#each feesChart.xLabels as lbl, i (i)}
                <text x={lbl.x} y={ACT_H - 10} class="x-label" text-anchor={lbl.anchor}>{lbl.label}</text>
              {/each}
              {#if hoveredActFee}
                {@const flip = hoveredActFee.x > ACT_W - 90}
                {@const tx   = flip ? hoveredActFee.x - 6 : hoveredActFee.x + 6}
                {@const anc  = flip ? 'end' : 'start'}
                <text x={tx} y={ACT_PAD.top + 12} class="hover-label" text-anchor={anc} fill="#e05252">{formatBal(hoveredActFee.fee)} sats</text>
                <text x={tx} y={ACT_PAD.top + 26} class="hover-label cf-date" text-anchor={anc}>{hoveredActFee.label} · {hoveredActFee.count} tx</text>
              {/if}
            </svg>
          </div>
          <div class="card-stats">
            <div><span class="stat-label">Total</span><span class="stat-value">{formatSummary(feesChart.totalFee)}</span></div>
            <div><span class="stat-label">Median / tx</span><span class="stat-value">{formatSummary(feesChart.medianFee)}</span></div>
            <div><span class="stat-label">{range === 'all' ? 'Txs' : 'Txs in range'}</span><span class="stat-value">{feesChart.totalTx.toLocaleString()}</span></div>
          </div>
        {/if}
      </section>
    </div>
  {/if}

  {#if view !== 'activity'}
  <div class="chart-stats">
    <div class="stat">
      <span class="stat-label">{range === 'all' ? 'Total received' : 'Period received'}</span>
      <span class="stat-value received">{formatSummary(summary.received)}</span>
    </div>
    <div class="stat">
      <span class="stat-label">{range === 'all' ? 'Total sent' : 'Period sent'}</span>
      <span class="stat-value sent">{formatSummary(summary.sent)}</span>
    </div>
    <div class="stat">
      <span class="stat-label">Net change</span>
      <!-- Neutral text color. The signed value (+/−) carries direction;
           reserving green/red exclusively for received/sent keeps the
           color palette unambiguous now that the chart's standalone
           legend is gone (the footer doubles as the color key). -->
      <span class="stat-value">{formatSummary(summary.net, true)}</span>
    </div>
  </div>
  {/if}
{/if}

<style>
  .empty { color: var(--text-muted); font-size: 0.9rem; padding: 24px 0; }
  .hidden-placeholder {
    padding: 32px 0; text-align: center;
    color: var(--text-muted); font-size: 0.85rem;
    border: 1px dashed var(--border); border-radius: 6px;
  }

  /* ── Controls ── */
  .controls {
    display: flex; align-items: center; justify-content: space-between;
    gap: 8px; margin-bottom: 6px; flex-wrap: wrap;
  }
  .range-group, .view-group { display: flex; gap: 4px; flex-wrap: wrap; align-items: center; }
  .ctrl-btn {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer;
    font-size: 0.72rem; font-weight: 600; letter-spacing: 0.04em;
    padding: 3px 9px;
    transition: color 0.1s, border-color 0.1s, background 0.1s;
  }
  .ctrl-btn:hover { color: var(--text); border-color: var(--text-muted); }
  .ctrl-btn.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: var(--accent); color: var(--accent);
  }

  .gran-select {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer;
    font-size: 0.72rem; font-weight: 600; letter-spacing: 0.04em;
    padding: 3px 22px 3px 9px; margin-left: 4px;
    appearance: none; -webkit-appearance: none;
    background-image: linear-gradient(45deg, transparent 50%, currentColor 50%),
                      linear-gradient(135deg, currentColor 50%, transparent 50%);
    background-position: calc(100% - 11px) 50%, calc(100% - 7px) 50%;
    background-size: 4px 4px, 4px 4px;
    background-repeat: no-repeat;
    transition: color 0.1s, border-color 0.1s;
  }
  .gran-select:hover  { color: var(--text); border-color: var(--text-muted); }
  .gran-select:focus  { outline: none; border-color: var(--accent); color: var(--accent); }
  .gran-select option { background: var(--surface-1); color: var(--text); }


  /* ── Chart ── */
  .chart-wrap { width: 100%; }
  .chart { width: 100%; height: auto; display: block; overflow: visible; }

  /* ── Activity view ── */
  .activity-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 24px;
    margin-top: 4px;
  }
  @media (max-width: 900px) {
    .activity-grid { grid-template-columns: 1fr; }
  }
  .activity-card { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .card-title {
    margin: 0; font-size: 0.82rem; font-weight: 600;
    color: var(--text); letter-spacing: 0.02em;
  }
  .card-stats {
    display: flex; flex-wrap: wrap; gap: 16px; margin-top: 4px;
    font-size: 0.74rem;
  }
  .card-stats > div { display: flex; flex-direction: column; gap: 1px; }
  .card-stats .stat-label {
    font-size: 0.66rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.05em;
  }
  .card-stats .stat-value {
    font-size: 0.8rem; font-weight: 600; color: var(--text);
    font-variant-numeric: tabular-nums;
  }

  /* ── Summary stats ── */
  .chart-stats {
    display: flex; gap: 32px; margin-top: 14px; flex-wrap: wrap;
    justify-content: center;
  }
  .stat { align-items: center; text-align: center; }
  .stat { display: flex; flex-direction: column; gap: 3px; }
  .stat-label { font-size: 0.72rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.05em; }
  .stat-value { font-size: 0.88rem; font-weight: 600; font-variant-numeric: tabular-nums; }
  .stat-value.received { color: #52a875; }
  .stat-value.sent { color: #e05252; }

  /* ── SVG text ── */
  .y-label {
    font-size: 9px; fill: var(--text-muted);
    text-anchor: end; dominant-baseline: middle;
    font-variant-numeric: tabular-nums;
  }
  .x-label { font-size: 9px; fill: var(--text-muted); }
  .bar-count {
    font-size: 10px; font-weight: 700; fill: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .today-label { fill: color-mix(in srgb, var(--accent) 60%, var(--text-muted)); }
  .hover-label { font-size: 9px; font-weight: 600; font-variant-numeric: tabular-nums; }
  .cf-date { fill: var(--text-muted); font-weight: 400; }

  /* ── Mobile ── */
  @media (max-width: 480px) {
    .controls { flex-direction: column; align-items: flex-start; }
  }
</style>
