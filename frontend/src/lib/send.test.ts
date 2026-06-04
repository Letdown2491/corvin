import { describe, it, expect } from 'vitest'
import {
  isSilentPaymentAddress,
  looksLikeAddressShape,
  uriHasPayjoin,
  looksLikeHrn,
  addressFromScan,
  chunkAddress,
  bip21AmountToField,
  addressShapeHint,
  showAddressEcho,
} from './send'

// A real-shaped mainnet bech32 address (≥14 chars, bc1 prefix).
const BC1 = 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4'
// A silent-payment-shaped address (sp1 prefix, long).
const SP1 = 'sp1q' + 'q'.repeat(113)

describe('isSilentPaymentAddress', () => {
  it('matches sp1/tsp1/sprt1 prefixes case-insensitively', () => {
    expect(isSilentPaymentAddress('sp1qxyz')).toBe(true)
    expect(isSilentPaymentAddress('TSP1QXYZ')).toBe(true)
    expect(isSilentPaymentAddress('sprt1qxyz')).toBe(true)
  })
  it('rejects non-SP and empty', () => {
    expect(isSilentPaymentAddress(BC1)).toBe(false)
    expect(isSilentPaymentAddress('')).toBe(false)
  })
})

describe('looksLikeAddressShape', () => {
  it('accepts bech32 mainnet/testnet/regtest and SP', () => {
    expect(looksLikeAddressShape(BC1)).toBe(true)
    expect(looksLikeAddressShape('tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxxxxxx')).toBe(true)
    expect(looksLikeAddressShape('bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kxxxx')).toBe(true)
    expect(looksLikeAddressShape(SP1)).toBe(true)
  })
  it('accepts a legacy base58 address', () => {
    expect(looksLikeAddressShape('1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2')).toBe(true)
  })
  it('rejects garbage, too-short, lightning, and oversized', () => {
    expect(looksLikeAddressShape('')).toBe(false)
    expect(looksLikeAddressShape('hello')).toBe(false)
    expect(looksLikeAddressShape('lnbc1pvjluezpp5...')).toBe(false)
    expect(looksLikeAddressShape('lightning:lnbc1...')).toBe(false)
    expect(looksLikeAddressShape('x'.repeat(201))).toBe(false)
  })
})

describe('uriHasPayjoin', () => {
  it('detects pj= in a bitcoin: URI', () => {
    expect(uriHasPayjoin(`bitcoin:${BC1}?pj=https://example.com/pj`)).toBe(true)
  })
  it('is false without pj or without bitcoin: scheme', () => {
    expect(uriHasPayjoin(`bitcoin:${BC1}?amount=0.1`)).toBe(false)
    expect(uriHasPayjoin(BC1)).toBe(false)
    expect(uriHasPayjoin(`bitcoin:${BC1}`)).toBe(false)
  })
})

describe('looksLikeHrn', () => {
  it('matches user@domain.tld, with or without ₿', () => {
    expect(looksLikeHrn('alice@example.com')).toBe(true)
    expect(looksLikeHrn('₿alice@example.com')).toBe(true)
  })
  it('rejects addresses, URIs, and incomplete names', () => {
    expect(looksLikeHrn(BC1)).toBe(false)
    expect(looksLikeHrn('bitcoin:alice@example.com')).toBe(false)
    expect(looksLikeHrn('alice@localhost')).toBe(false)
    expect(looksLikeHrn('alice')).toBe(false)
  })
})

describe('addressFromScan', () => {
  it('extracts a bare address', () => {
    expect(addressFromScan(BC1)).toEqual({ address: BC1, amount: null })
  })
  it('extracts address + amount from a BIP21 URI', () => {
    const r = addressFromScan(`bitcoin:${BC1}?amount=0.001`)
    expect(r?.address).toBe(BC1)
    expect(r?.amount).toBe('0.001')
  })
  it('handles a bitcoin: prefix without BIP21 params', () => {
    expect(addressFromScan(`bitcoin:${BC1}`)?.address).toBe(BC1)
  })
  it('returns null for non-address payloads', () => {
    expect(addressFromScan('https://example.com')).toBeNull()
    expect(addressFromScan('garbage')).toBeNull()
  })
})

describe('chunkAddress', () => {
  it('splits into 4-char chunks', () => {
    expect(chunkAddress('abcdefghij')).toEqual(['abcd', 'efgh', 'ij'])
  })
  it('trims and returns [] for empty', () => {
    expect(chunkAddress('   ')).toEqual([])
    expect(chunkAddress('')).toEqual([])
  })
})

describe('bip21AmountToField', () => {
  it('passes BTC through unchanged when unit is btc', () => {
    expect(bip21AmountToField('0.001', 'btc')).toBe('0.001')
  })
  it('converts BTC to sats when unit is sats', () => {
    expect(bip21AmountToField('0.001', 'sats')).toBe('100000')
  })
  it('falls back to the original string when conversion fails', () => {
    expect(bip21AmountToField('not-a-number', 'sats')).toBe('not-a-number')
  })
})

describe('addressShapeHint', () => {
  it('classifies empty / ok / bad', () => {
    expect(addressShapeHint('')).toBe('empty')
    expect(addressShapeHint('   ')).toBe('empty')
    expect(addressShapeHint(BC1)).toBe('ok')
    expect(addressShapeHint('clearly-not-an-address')).toBe('bad')
  })
})

describe('showAddressEcho', () => {
  it('shows for a plausible address', () => {
    expect(showAddressEcho(BC1)).toBe(true)
  })
  it('hides while mid-type, for HRNs, and for bad shapes', () => {
    expect(showAddressEcho('bc1qshort')).toBe(false) // < 14 chars
    expect(showAddressEcho('alice@example.com')).toBe(false) // HRN
    expect(showAddressEcho('this-is-long-but-not-an-address')).toBe(false) // bad shape
  })
})
