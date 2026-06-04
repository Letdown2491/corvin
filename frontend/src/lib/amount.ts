// Money parsing/formatting, extracted from SendModal so it's pure + unit-tested.
// The key concern: convert a BTC decimal string to integer sats WITHOUT
// `parseFloat(s) * 1e8`, which loses precision for some 8-dp values. We parse the
// integer and fractional parts as strings and combine them as integers.
//
// Note: the server (BDK) is authoritative on the real sat amount of a send; these
// helpers drive what the UI displays/sends, so the goal is exactness + consistency.

/** 21,000,000 BTC in sats — the cap any valid amount must stay under. */
export const MAX_MONEY_SATS = 21_000_000 * 100_000_000

/**
 * Parse a BTC decimal string to integer sats. Returns `null` for anything
 * invalid: non-numeric, negative, more than 8 decimal places, or above
 * MAX_MONEY. No floating point is used for the conversion itself.
 */
export function btcToSats(input: string): number | null {
  const s = input.trim()
  // Optional leading +, digits, optional fractional part. No exponent, no sign-.
  if (!/^\+?\d*\.?\d+$/.test(s) && !/^\+?\d+\.?\d*$/.test(s)) return null
  const cleaned = s.replace(/^\+/, '')
  const [intPart, fracPartRaw = ''] = cleaned.split('.')
  if (fracPartRaw.length > 8) return null // more precision than sats can hold
  const frac = fracPartRaw.padEnd(8, '0')
  const intSats = Number(intPart || '0') * 100_000_000
  const fracSats = Number(frac)
  if (!Number.isFinite(intSats) || !Number.isFinite(fracSats)) return null
  const total = intSats + fracSats
  if (!Number.isSafeInteger(total) || total < 0 || total > MAX_MONEY_SATS) return null
  return total
}

/** Parse a sats string to an integer number of sats, or null if invalid. */
export function satsStringToSats(input: string): number | null {
  const s = input.trim()
  if (!/^\d+$/.test(s)) return null
  const n = Number(s)
  if (!Number.isSafeInteger(n) || n > MAX_MONEY_SATS) return null
  return n
}

/**
 * Convert a user-entered amount in the given display unit to integer sats.
 * Returns 0 for empty / invalid / non-positive input (matching the prior
 * SendModal behaviour, where 0 means "nothing to send from this row").
 */
export function amountToSats(amount: string, unit: 'btc' | 'sats'): number {
  const parsed = unit === 'btc' ? btcToSats(amount) : satsStringToSats(amount)
  return parsed && parsed > 0 ? parsed : 0
}

/** Format integer sats as a BTC decimal string (no trailing zeros, no unit). */
export function satsToBtcString(sats: number): string {
  const sign = sats < 0 ? '-' : ''
  const abs = Math.abs(sats)
  const whole = Math.floor(abs / 100_000_000)
  const frac = (abs % 100_000_000).toString().padStart(8, '0').replace(/0+$/, '')
  return frac ? `${sign}${whole}.${frac}` : `${sign}${whole}`
}

/**
 * Parse a BIP-21 `bitcoin:<addr>?amount=<btc>` URI. Returns the address and the
 * raw amount string (in BTC, per BIP-21), or null if it isn't a bitcoin: URI.
 */
export function parseBip21(input: string): { address: string; amount: string | null } | null {
  const trimmed = input.trim()
  if (!/^bitcoin:/i.test(trimmed)) return null
  const rest = trimmed.slice('bitcoin:'.length)
  const [addr, q] = rest.split('?', 2)
  if (!addr) return null
  let amount: string | null = null
  if (q) {
    const a = new URLSearchParams(q).get('amount')
    if (a && btcToSats(a) !== null) amount = a
  }
  return { address: addr, amount }
}
