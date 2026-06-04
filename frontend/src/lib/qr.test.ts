import { describe, it, expect } from 'vitest'
import { encodeCborByteString, decodeCborByteString } from './qr'

describe('CBOR byte-string round-trip', () => {
  // Covers each length-encoding boundary the encoder switches on.
  for (const n of [0, 1, 23, 24, 255, 256, 1000, 65535, 65536]) {
    it(`round-trips a ${n}-byte payload`, () => {
      const data = new Uint8Array(n).map((_, i) => i & 0xff)
      const decoded = decodeCborByteString(encodeCborByteString(data))
      expect(decoded).not.toBeNull()
      expect(Array.from(decoded!)).toEqual(Array.from(data))
    })
  }

  it('decodes a byte string wrapped in a leading CBOR tag', () => {
    // tag(24) header (0xd8 0x18) then a 2-byte string 0x42 0xaa 0xbb.
    const buf = new Uint8Array([0xd8, 0x18, 0x42, 0xaa, 0xbb])
    expect(Array.from(decodeCborByteString(buf)!)).toEqual([0xaa, 0xbb])
  })
})

describe('CBOR decode hygiene (untrusted QR input)', () => {
  it('rejects an absurd declared length instead of allocating', () => {
    // 0x5a = byte string, 4-byte length, declaring ~4.29 GB with no payload.
    const buf = new Uint8Array([0x5a, 0xff, 0xff, 0xff, 0xff])
    expect(decodeCborByteString(buf)).toBeNull()
  })

  it('reads the 32-bit length as unsigned (high bit set does not go negative)', () => {
    // Same high-bit-set length; the guard must treat it as huge → null, never as
    // a negative length that slips past the bounds check.
    const buf = new Uint8Array([0x5a, 0x80, 0x00, 0x00, 0x00, 0x01, 0x02])
    expect(decodeCborByteString(buf)).toBeNull()
  })

  it('rejects a header whose length bytes are truncated', () => {
    // Claims a 2-byte length but only one byte follows.
    expect(decodeCborByteString(new Uint8Array([0x59, 0x01]))).toBeNull()
  })

  it('rejects a declared length longer than the buffer', () => {
    // Says 10 bytes, supplies 2.
    expect(decodeCborByteString(new Uint8Array([0x4a, 0x01, 0x02]))).toBeNull()
  })

  it('returns null for a non-byte-string major type', () => {
    // 0x00 = unsigned int major type, not a byte string.
    expect(decodeCborByteString(new Uint8Array([0x00]))).toBeNull()
  })

  it('returns null for an empty buffer', () => {
    expect(decodeCborByteString(new Uint8Array([]))).toBeNull()
  })
})
