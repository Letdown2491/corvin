import type { TxRecord } from './types'

const MONTH_MAP: Record<string, number> = {
  january: 0, jan: 0,
  february: 1, feb: 1,
  march: 2, mar: 2,
  april: 3, apr: 3,
  may: 4,
  june: 5, jun: 5,
  july: 6, jul: 6,
  august: 7, aug: 7,
  september: 8, sep: 8, sept: 8,
  october: 9, oct: 9,
  november: 10, nov: 10,
  december: 11, dec: 11,
}
const MONTH_RE = Object.keys(MONTH_MAP).join('|')

const STOP_WORDS = new Set([
  'a', 'an', 'the', 'i', 'me', 'my', 'we', 'our', 'you', 'your',
  'he', 'she', 'it', 'his', 'her', 'its', 'they', 'their', 'them',
  'is', 'are', 'was', 'were', 'be', 'been', 'being',
  'have', 'has', 'had', 'do', 'does', 'did',
  'will', 'would', 'could', 'should', 'may', 'might', 'shall', 'can',
  'this', 'that', 'these', 'those', 'what', 'which', 'who', 'where', 'when', 'how',
  'and', 'or', 'but', 'so', 'if', 'then', 'not',
  'of', 'in', 'on', 'at', 'to', 'for', 'with', 'by', 'from', 'up', 'into', 'about', 'since',
  'one', 'all', 'just', 'only', 'any', 'some', 'more', 'most', 'no',
  'get', 'got', 'pay', 'paid', 'send', 'sent', 'receive', 'received',
  'buy', 'bought', 'transfer', 'made', 'make', 'show', 'showing',
  'transaction', 'transactions', 'tx', 'wallet',
  'recent', 'recently', 'last', 'ago', 'past',
])

export interface ParsedQuery {
  direction?: 'sent' | 'received'
  amountMin?: number   // sats
  amountMax?: number   // sats
  dateFrom?: Date
  dateTo?: Date
  status?: 'unconfirmed' | 'confirmed'
  labelKeywords: string[]
  summary: string
}

function startOfDay(d: Date): Date {
  const r = new Date(d); r.setHours(0, 0, 0, 0); return r
}
function startOfWeek(d: Date): Date {
  const r = startOfDay(d); r.setDate(r.getDate() - r.getDay()); return r
}
function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1)
}
function startOfYear(d: Date): Date {
  return new Date(d.getFullYear(), 0, 1)
}
function subDays(d: Date, n: number): Date {
  return new Date(d.getTime() - n * 86_400_000)
}

function parseSats(value: string, unit: string | undefined): number {
  const n = parseFloat(value)
  if (isNaN(n)) return 0
  if (unit && /^(btc|bitcoin)$/i.test(unit)) return Math.round(n * 1e8)
  return Math.round(n)
}

function fmtSats(sats: number): string {
  if (sats >= 1e8) return `${(sats / 1e8).toFixed(8).replace(/\.?0+$/, '')} BTC`
  return `${sats.toLocaleString()} sats`
}

function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

export function parseQuery(raw: string, now = new Date()): ParsedQuery {
  let s = raw.toLowerCase().trim()
  const parts: string[] = []
  const result: ParsedQuery = { labelKeywords: [], summary: '' }

  // ── Direction ──────────────────────────────────────────────────────────
  if (/\b(sends?|sent|outgoing)\b/.test(s)) {
    result.direction = 'sent'; parts.push('sent')
    s = s.replace(/\b(sends?|sent|outgoing)\b/g, ' ')
  } else if (/\b(receives?|received|incoming)\b/.test(s)) {
    result.direction = 'received'; parts.push('received')
    s = s.replace(/\b(receives?|received|incoming)\b/g, ' ')
  }

  // ── Status ─────────────────────────────────────────────────────────────
  if (/\b(unconfirmed|pending|mempool)\b/.test(s)) {
    result.status = 'unconfirmed'; parts.push('unconfirmed')
    s = s.replace(/\b(unconfirmed|pending|mempool)\b/g, ' ')
  } else if (/\bconfirmed\b/.test(s)) {
    result.status = 'confirmed'; parts.push('confirmed')
    s = s.replace(/\bconfirmed\b/g, ' ')
  }

  // ── Amount ─────────────────────────────────────────────────────────────
  const amtUnit = '(btc|bitcoin|sats?|satoshis?)?'
  const amtVal = '(\\d+(?:\\.\\d+)?)'

  const betweenM = s.match(new RegExp(`between\\s+${amtVal}\\s*${amtUnit}\\s+and\\s+${amtVal}\\s*${amtUnit}`))
  if (betweenM) {
    const unit = betweenM[2] || betweenM[4]
    result.amountMin = parseSats(betweenM[1], unit)
    result.amountMax = parseSats(betweenM[3], betweenM[4] || unit)
    parts.push(`${fmtSats(result.amountMin)} – ${fmtSats(result.amountMax)}`)
    s = s.replace(betweenM[0], ' ')
  } else {
    const moreM = s.match(new RegExp(`(?:more\\s+than|over|greater\\s+than|above|at\\s+least|>=?)\\s*${amtVal}\\s*${amtUnit}`))
    if (moreM) {
      result.amountMin = parseSats(moreM[1], moreM[2])
      parts.push(`> ${fmtSats(result.amountMin)}`)
      s = s.replace(moreM[0], ' ')
    }
    const lessM = s.match(new RegExp(`(?:less\\s+than|under|below|at\\s+most|<=?)\\s*${amtVal}\\s*${amtUnit}`))
    if (lessM) {
      result.amountMax = parseSats(lessM[1], lessM[2])
      parts.push(`< ${fmtSats(result.amountMax)}`)
      s = s.replace(lessM[0], ' ')
    }
  }

  // ── Explicit label ─────────────────────────────────────────────────────
  const hasLabelM = s.match(/\b(?:has\s+(?:a\s+)?(?:label|note|tag)|is\s+(?:labeled|tagged))\b/)
  if (hasLabelM) {
    result.labelKeywords = ['*']
    parts.push('has label')
    s = s.replace(hasLabelM[0], ' ')
  } else {
    const labelM = s.match(/\b(?:label(?:ed)?|tag(?:ged)?|note)\s+(.+)/)
    if (labelM) {
      const kw = labelM[1].trim()
      result.labelKeywords = [kw]
      parts.push(`labeled "${kw}"`)
      s = s.replace(labelM[0], ' ')
    }
  }

  // ── Dates ─────────────────────────────────────────────────────────────

  // "from <month> [year] to <month> [year]" / "<month> to <month> [year]"
  if (!result.dateFrom) {
    const fromToRe = new RegExp(
      `(?:from\\s+)?(${MONTH_RE})\\s*(\\d{4})?\\s+(?:to|through|until)\\s+(${MONTH_RE})\\s*(\\d{4})?`
    )
    const m = s.match(fromToRe)
    if (m) {
      const y2 = m[4] ? parseInt(m[4]) : now.getFullYear()
      const y1 = m[2] ? parseInt(m[2]) : y2
      result.dateFrom = new Date(y1, MONTH_MAP[m[1]], 1)
      result.dateTo = new Date(y2, MONTH_MAP[m[3]] + 1, 1)
      parts.push(`${cap(m[1])} ${y1} – ${cap(m[3])} ${y2}`)
      s = s.replace(m[0], ' ')
    }
  }

  // "last N days/weeks/months"
  if (!result.dateFrom) {
    const m = s.match(/last\s+(\d+)\s+(days?|weeks?|months?)/)
    if (m) {
      const n = parseInt(m[1]), u = m[2]
      result.dateTo = now
      if (u.startsWith('day')) result.dateFrom = subDays(now, n)
      else if (u.startsWith('week')) result.dateFrom = subDays(now, n * 7)
      else result.dateFrom = new Date(now.getFullYear(), now.getMonth() - n, 1)
      parts.push(`last ${n} ${u}`)
      s = s.replace(m[0], ' ')
    }
  }

  // Named relative ranges
  if (!result.dateFrom) {
    if (/\btoday\b/.test(s)) {
      result.dateFrom = startOfDay(now); result.dateTo = now
      parts.push('today'); s = s.replace(/\btoday\b/, ' ')
    } else if (/\byesterday\b/.test(s)) {
      result.dateFrom = startOfDay(subDays(now, 1)); result.dateTo = startOfDay(now)
      parts.push('yesterday'); s = s.replace(/\byesterday\b/, ' ')
    } else if (/\bthis\s+week\b/.test(s)) {
      result.dateFrom = startOfWeek(now); result.dateTo = now
      parts.push('this week'); s = s.replace(/\bthis\s+week\b/, ' ')
    } else if (/\blast\s+week\b/.test(s)) {
      const sw = startOfWeek(now)
      result.dateTo = sw; result.dateFrom = subDays(sw, 7)
      parts.push('last week'); s = s.replace(/\blast\s+week\b/, ' ')
    } else if (/\bthis\s+month\b/.test(s)) {
      result.dateFrom = startOfMonth(now); result.dateTo = now
      parts.push('this month'); s = s.replace(/\bthis\s+month\b/, ' ')
    } else if (/\blast\s+month\b/.test(s)) {
      const sm = startOfMonth(now)
      result.dateTo = sm; result.dateFrom = new Date(now.getFullYear(), now.getMonth() - 1, 1)
      parts.push('last month'); s = s.replace(/\blast\s+month\b/, ' ')
    } else if (/\bthis\s+year\b/.test(s)) {
      result.dateFrom = startOfYear(now); result.dateTo = now
      parts.push('this year'); s = s.replace(/\bthis\s+year\b/, ' ')
    } else if (/\blast\s+year\b/.test(s)) {
      const sy = startOfYear(now)
      result.dateTo = sy; result.dateFrom = new Date(now.getFullYear() - 1, 0, 1)
      parts.push('last year'); s = s.replace(/\blast\s+year\b/, ' ')
    } else if (/\b(recent(?:ly)?|not\s+(?:too\s+)?long\s+ago)\b/.test(s)) {
      result.dateFrom = subDays(now, 30); result.dateTo = now
      parts.push('last 30 days'); s = s.replace(/\b(recent(?:ly)?|not\s+(?:too\s+)?long\s+ago)\b/, ' ')
    }
  }

  // Quarter: "q1 2026" / "q3"
  if (!result.dateFrom) {
    const m = s.match(/\bq([1-4])\b\s*(\d{4})?/)
    if (m) {
      const q = parseInt(m[1]) - 1
      const year = m[2] ? parseInt(m[2]) : now.getFullYear()
      result.dateFrom = new Date(year, q * 3, 1)
      result.dateTo = new Date(year, q * 3 + 3, 1)
      parts.push(`Q${m[1]} ${year}`)
      s = s.replace(m[0], ' ')
    }
  }

  // "since/from <month> [year]" — open-ended start
  if (!result.dateFrom) {
    const sinceRe = new RegExp(`(?:since|from)\\s+(${MONTH_RE})\\s*(\\d{4})?`)
    const m = s.match(sinceRe)
    if (m) {
      const year = m[2] ? parseInt(m[2]) : now.getFullYear()
      result.dateFrom = new Date(year, MONTH_MAP[m[1]], 1)
      parts.push(`since ${cap(m[1])} ${year}`)
      s = s.replace(m[0], ' ')
    }
  }

  // "<month> [year]" or bare 4-digit year
  if (!result.dateFrom) {
    const monthYearRe = new RegExp(`\\b(${MONTH_RE})\\s*(\\d{4})?\\b`)
    const m = s.match(monthYearRe)
    if (m) {
      const year = m[2] ? parseInt(m[2]) : now.getFullYear()
      result.dateFrom = new Date(year, MONTH_MAP[m[1]], 1)
      result.dateTo = new Date(year, MONTH_MAP[m[1]] + 1, 1)
      parts.push(m[2] ? `${cap(m[1])} ${year}` : cap(m[1]))
      s = s.replace(m[0], ' ')
    } else {
      const yearM = s.match(/\b(20[0-9]{2})\b/)
      if (yearM) {
        const year = parseInt(yearM[1])
        result.dateFrom = new Date(year, 0, 1)
        result.dateTo = new Date(year + 1, 0, 1)
        parts.push(yearM[1])
        s = s.replace(yearM[0], ' ')
      }
    }
  }

  // ── Fallback: keyword search across labels + txids ─────────────────────
  if (result.labelKeywords.length === 0) {
    const keywords = s.split(/\s+/).filter(w =>
      w.length > 2 && !STOP_WORDS.has(w) && !/^\d+$/.test(w)
    )
    if (keywords.length > 0) {
      result.labelKeywords = keywords
      if (parts.length === 0) parts.push(`"${keywords.join(' ')}"`)
    }
  }

  result.summary = parts.join(' · ')
  return result
}

export function filterTxs(
  txs: TxRecord[],
  parsed: ParsedQuery,
  labels: Record<string, string>,
): TxRecord[] {
  return txs.filter(tx => {
    if (parsed.direction === 'sent' && tx.amount_sats >= 0) return false
    if (parsed.direction === 'received' && tx.amount_sats < 0) return false

    if (parsed.status === 'unconfirmed' && tx.confirmations !== 0) return false
    if (parsed.status === 'confirmed' && tx.confirmations === 0) return false

    const abs = Math.abs(tx.amount_sats)
    if (parsed.amountMin !== undefined && abs < parsed.amountMin) return false
    if (parsed.amountMax !== undefined && abs > parsed.amountMax) return false

    if (parsed.dateFrom || parsed.dateTo) {
      if (!tx.timestamp) return false
      const ts = new Date(tx.timestamp)
      if (parsed.dateFrom && ts < parsed.dateFrom) return false
      if (parsed.dateTo && ts >= parsed.dateTo) return false
    }

    if (parsed.labelKeywords.length > 0) {
      const label = (labels[tx.txid] ?? '').toLowerCase()
      const txid = tx.txid.toLowerCase()
      if (parsed.labelKeywords[0] === '*') {
        if (!label) return false
      } else {
        if (!parsed.labelKeywords.every(kw => label.includes(kw) || txid.includes(kw))) return false
      }
    }

    return true
  })
}
