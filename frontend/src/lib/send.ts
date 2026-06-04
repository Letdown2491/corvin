// Pure address/URI parsing + display helpers for the send flow. Extracted from
// SendModal.svelte so the trickiest recipient-input logic is unit-testable and
// free of component state. The backend still validates fully; these are
// client-side hints + formatting.
import { parseBip21, btcToSats } from './amount'

export function isSilentPaymentAddress(s: string): boolean {
  if (!s) return false
  const lower = s.toLowerCase()
  return lower.startsWith('sp1') || lower.startsWith('tsp1') || lower.startsWith('sprt1')
}

/// Shape check. Catches the "user pasted garbage" case before they wait for a
/// build_tx round-trip. Not a validator — the backend is authoritative.
export function looksLikeAddressShape(s: string): boolean {
  if (!s) return false
  if (s.length < 14 || s.length > 200) return false // SP addresses are ~117 chars
  const lower = s.toLowerCase()
  if (lower.startsWith('lightning:') || lower.startsWith('ln')) return false
  if (isSilentPaymentAddress(s)) return true
  if (lower.startsWith('bc1') || lower.startsWith('tb1') || lower.startsWith('bcrt1')) return true
  return /^[123mn][1-9A-HJ-NP-Za-km-z]{20,}$/.test(s)
}

/// True if a BIP-21 URI carries a payjoin (`pj=`) endpoint.
export function uriHasPayjoin(input: string): boolean {
  const t = input.trim()
  if (!/^bitcoin:/i.test(t)) return false
  const q = t.slice(8).split('?', 2)[1]
  return q ? new URLSearchParams(q).has('pj') : false
}

/// Looks like a BIP-353 human-readable name (₿user@domain), not a bitcoin: URI.
export function looksLikeHrn(s: string): boolean {
  const t = s.trim().replace(/^₿/, '')
  return /^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(t) && !t.toLowerCase().startsWith('bitcoin:')
}

/// Pull a recipient address (and optional amount) out of a scanned/pasted
/// payload. Returns null when it isn't address-shaped. Handles BIP21 URIs and
/// bare/Silent Payment addresses.
export function addressFromScan(raw: string): { address: string; amount: string | null } | null {
  const parsed = parseBip21(raw)
  const address = parsed
    ? parsed.address
    : raw.toLowerCase().startsWith('bitcoin:')
      ? raw.slice(8).split('?')[0]
      : raw
  if (!address || !looksLikeAddressShape(address)) return null
  return { address, amount: parsed?.amount ?? null }
}

/// Group an address into 4-char chunks for a legible, verifiable echo.
export function chunkAddress(addr: string): string[] {
  const a = addr.trim()
  return a.length ? (a.match(/.{1,4}/g) ?? [a]) : []
}

/// Convert a BIP21 amount (always BTC) into the string the recipient field
/// expects given the current display unit.
export function bip21AmountToField(btcAmount: string, unit: 'btc' | 'sats'): string {
  if (unit !== 'sats') return btcAmount
  const sats = btcToSats(btcAmount)
  return sats === null ? btcAmount : sats.toString()
}

/// Traffic-light hint for a recipient address field.
export function addressShapeHint(addr: string): 'ok' | 'bad' | 'empty' {
  const a = addr.trim()
  if (!a) return 'empty'
  return looksLikeAddressShape(a) ? 'ok' : 'bad'
}

/// Show the chunked-address echo only once the input plausibly holds a real
/// address (not mid-type, not an HRN we'll resolve, not garbage).
export function showAddressEcho(addr: string): boolean {
  const a = addr.trim()
  return a.length >= 14 && addressShapeHint(a) !== 'bad' && !looksLikeHrn(a)
}
