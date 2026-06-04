import { describe, it, expect } from 'vitest'
import { btcToSats, satsStringToSats, amountToSats, satsToBtcString, parseBip21, MAX_MONEY_SATS } from './amount'

describe('btcToSats', () => {
  it('converts whole and fractional BTC exactly', () => {
    expect(btcToSats('1')).toBe(100_000_000)
    expect(btcToSats('0.5')).toBe(50_000_000)
    expect(btcToSats('0.00000001')).toBe(1)
    expect(btcToSats('21000000')).toBe(MAX_MONEY_SATS)
  })

  it('handles the float-trap values exactly (the F-5 motivation)', () => {
    // 21.4 * 1e8 in float64 is 2139999999.9999998 → Math.round saves it, but the
    // string path is exact regardless.
    expect(btcToSats('21.4')).toBe(2_140_000_000)
    // 0.1 + 0.2 style: each parsed independently and exactly.
    expect(btcToSats('0.1')).toBe(10_000_000)
    expect(btcToSats('0.2')).toBe(20_000_000)
    expect(btcToSats('0.3')).toBe(30_000_000)
    // A value with all 8 dp populated.
    expect(btcToSats('1.23456789')).toBe(123_456_789)
  })

  it('accepts forms with leading/trailing dot', () => {
    expect(btcToSats('.5')).toBe(50_000_000)
    expect(btcToSats('1.')).toBe(100_000_000)
  })

  it('rejects more than 8 decimal places', () => {
    expect(btcToSats('0.000000001')).toBeNull()
  })

  it('rejects negatives, non-numeric, NaN/Infinity strings', () => {
    expect(btcToSats('-1')).toBeNull()
    expect(btcToSats('abc')).toBeNull()
    expect(btcToSats('1e8')).toBeNull()
    expect(btcToSats('Infinity')).toBeNull()
    expect(btcToSats('')).toBeNull()
  })

  it('rejects amounts over MAX_MONEY', () => {
    expect(btcToSats('21000001')).toBeNull()
  })
})

describe('satsStringToSats', () => {
  it('parses integer sats', () => {
    expect(satsStringToSats('1000')).toBe(1000)
    expect(satsStringToSats('0')).toBe(0)
  })
  it('rejects decimals and junk', () => {
    expect(satsStringToSats('1000.5')).toBeNull()
    expect(satsStringToSats('-5')).toBeNull()
    expect(satsStringToSats('abc')).toBeNull()
  })
  it('rejects over MAX_MONEY', () => {
    expect(satsStringToSats(String(MAX_MONEY_SATS + 1))).toBeNull()
  })
})

describe('amountToSats (unit-aware, 0 for invalid — prior behaviour)', () => {
  it('btc unit', () => {
    expect(amountToSats('0.5', 'btc')).toBe(50_000_000)
  })
  it('sats unit', () => {
    expect(amountToSats('12345', 'sats')).toBe(12345)
  })
  it('returns 0 for empty / invalid / non-positive', () => {
    expect(amountToSats('', 'btc')).toBe(0)
    expect(amountToSats('0', 'btc')).toBe(0)
    expect(amountToSats('abc', 'sats')).toBe(0)
    expect(amountToSats('-1', 'btc')).toBe(0)
  })
})

describe('satsToBtcString', () => {
  it('formats with no trailing zeros', () => {
    expect(satsToBtcString(100_000_000)).toBe('1')
    expect(satsToBtcString(50_000_000)).toBe('0.5')
    expect(satsToBtcString(1)).toBe('0.00000001')
    expect(satsToBtcString(123_456_789)).toBe('1.23456789')
  })
  it('round-trips with btcToSats', () => {
    for (const s of [1, 50_000_000, 123_456_789, 2_140_000_000, MAX_MONEY_SATS]) {
      expect(btcToSats(satsToBtcString(s))).toBe(s)
    }
  })
})

describe('parseBip21', () => {
  it('parses address + amount', () => {
    expect(parseBip21('bitcoin:bc1qexample?amount=0.5')).toEqual({ address: 'bc1qexample', amount: '0.5' })
  })
  it('parses address with no amount', () => {
    expect(parseBip21('bitcoin:bc1qexample')).toEqual({ address: 'bc1qexample', amount: null })
  })
  it('ignores an invalid amount param', () => {
    expect(parseBip21('bitcoin:bc1qexample?amount=notanumber')?.amount).toBeNull()
  })
  it('is case-insensitive on the scheme', () => {
    expect(parseBip21('BITCOIN:bc1q?amount=1')?.address).toBe('bc1q')
  })
  it('returns null for non-bitcoin input', () => {
    expect(parseBip21('bc1qexample')).toBeNull()
    expect(parseBip21('https://x')).toBeNull()
  })
})
