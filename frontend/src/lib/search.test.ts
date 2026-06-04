import { describe, it, expect } from 'vitest'
import { parseQuery } from './search'

// Fixed reference date for deterministic relative-date assertions:
// Wed 2026-05-20 12:00 local.
const NOW = new Date(2026, 4, 20, 12, 0, 0)

describe('parseQuery — direction', () => {
  it('detects sent', () => {
    expect(parseQuery('sent', NOW).direction).toBe('sent')
    expect(parseQuery('outgoing payments', NOW).direction).toBe('sent')
  })
  it('detects received', () => {
    expect(parseQuery('received', NOW).direction).toBe('received')
    expect(parseQuery('incoming', NOW).direction).toBe('received')
  })
  it('leaves direction unset when absent', () => {
    expect(parseQuery('groceries', NOW).direction).toBeUndefined()
  })
})

describe('parseQuery — status', () => {
  it('detects unconfirmed/pending/mempool', () => {
    expect(parseQuery('pending', NOW).status).toBe('unconfirmed')
    expect(parseQuery('mempool', NOW).status).toBe('unconfirmed')
  })
  it('detects confirmed', () => {
    expect(parseQuery('confirmed', NOW).status).toBe('confirmed')
  })
})

describe('parseQuery — amounts', () => {
  it('parses "more than" in sats (default unit)', () => {
    const q = parseQuery('more than 50000', NOW)
    expect(q.amountMin).toBe(50000)
    expect(q.amountMax).toBeUndefined()
  })
  it('parses "over N btc" → sats', () => {
    const q = parseQuery('over 1 btc', NOW)
    expect(q.amountMin).toBe(100_000_000)
  })
  it('parses "less than" / under', () => {
    expect(parseQuery('under 1000 sats', NOW).amountMax).toBe(1000)
  })
  it('parses a between range', () => {
    const q = parseQuery('between 0.5 and 1 btc', NOW)
    expect(q.amountMin).toBe(50_000_000)
    expect(q.amountMax).toBe(100_000_000)
  })
  it('can combine min and max', () => {
    const q = parseQuery('over 1000 sats under 5000 sats', NOW)
    expect(q.amountMin).toBe(1000)
    expect(q.amountMax).toBe(5000)
  })
})

describe('parseQuery — labels', () => {
  it('detects "has a label" as wildcard', () => {
    expect(parseQuery('has a label', NOW).labelKeywords).toEqual(['*'])
  })
  it('captures an explicit label keyword', () => {
    expect(parseQuery('labeled coffee', NOW).labelKeywords).toEqual(['coffee'])
  })
})

describe('parseQuery — dates', () => {
  it('today → from start-of-day to now', () => {
    const q = parseQuery('today', NOW)
    expect(q.dateFrom?.getDate()).toBe(20)
    expect(q.dateFrom?.getHours()).toBe(0)
    expect(q.dateTo).toEqual(NOW)
  })
  it('last 7 days', () => {
    const q = parseQuery('last 7 days', NOW)
    expect(q.dateFrom).toBeInstanceOf(Date)
    // 7 days before NOW
    expect(Math.round((NOW.getTime() - q.dateFrom!.getTime()) / 86_400_000)).toBe(7)
  })
  it('month range "jan to mar 2026"', () => {
    const q = parseQuery('jan to mar 2026', NOW)
    expect(q.dateFrom).toEqual(new Date(2026, 0, 1))
    // dateTo is exclusive start of the month after March
    expect(q.dateTo).toEqual(new Date(2026, 3, 1))
  })
})

describe('parseQuery — combined + summary', () => {
  it('combines direction + amount + label and builds a summary', () => {
    const q = parseQuery('sent over 1 btc labeled rent', NOW)
    expect(q.direction).toBe('sent')
    expect(q.amountMin).toBe(100_000_000)
    expect(q.labelKeywords).toEqual(['rent'])
    expect(q.summary.length).toBeGreaterThan(0)
  })
  it('empty query yields empty parse', () => {
    const q = parseQuery('', NOW)
    expect(q.direction).toBeUndefined()
    expect(q.amountMin).toBeUndefined()
    expect(q.labelKeywords).toEqual([])
  })
})
