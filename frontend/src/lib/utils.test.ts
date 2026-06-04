import { describe, it, expect } from 'vitest'
import { base64ToBytes, bytesToBase64, utxoKey, kindLabel } from './utils'

describe('base64 round-trip', () => {
  it('round-trips arbitrary bytes', () => {
    const bytes = new Uint8Array([0, 1, 2, 254, 255, 127, 128])
    expect(bytesToBase64(bytes)).toBeTypeOf('string')
    expect(Array.from(base64ToBytes(bytesToBase64(bytes)))).toEqual(Array.from(bytes))
  })

  it('handles empty input', () => {
    expect(bytesToBase64(new Uint8Array([]))).toBe('')
    expect(base64ToBytes('').length).toBe(0)
  })

  it('decodes a known base64 value', () => {
    // "hi" → aGk=
    expect(bytesToBase64(new Uint8Array([0x68, 0x69]))).toBe('aGk=')
    expect(Array.from(base64ToBytes('aGk='))).toEqual([0x68, 0x69])
  })
})

describe('utxoKey', () => {
  it('joins txid and vout with a colon', () => {
    expect(utxoKey('abcd', 0)).toBe('abcd:0')
    expect(utxoKey('deadbeef', 3)).toBe('deadbeef:3')
  })
})

describe('kindLabel', () => {
  it('maps known kinds to user-facing labels', () => {
    expect(kindLabel('address')).toBe('Watch address')
    expect(kindLabel('taproot')).toBe('HD wallet')
    expect(kindLabel('zpub')).toBe('HD wallet')
    expect(kindLabel('multisig')).toBe('Multisig')
    expect(kindLabel('silent_payments')).toBe('Silent Payments')
    expect(kindLabel('descriptor')).toBe('Imported descriptor')
  })

  it('falls back to the raw kind for unknown values', () => {
    expect(kindLabel('something_new')).toBe('something_new')
  })
})
