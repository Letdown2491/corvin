// Lightweight client-side BIP39 validation. Catches typos and bad checksums
// before the user submits, instead of waiting for the backend to reject.
//
// We don't derive seeds here (the server does that, via the `bip39` crate).
// We just need: (1) every word is in the official list, (2) the embedded
// checksum at the end of the mnemonic matches.

import { BIP39_WORDS, BIP39_WORD_SET } from './bip39_words'

export type MnemonicCheck =
  | { kind: 'ok' }
  | { kind: 'wrong_count'; count: number }
  | { kind: 'invalid_words'; words: string[] }
  | { kind: 'bad_checksum' }

const VALID_COUNTS = new Set([12, 15, 18, 21, 24])

export function normalizeMnemonic(input: string): string[] {
  return input.trim().toLowerCase().split(/\s+/).filter(Boolean)
}

export function checkWordlist(words: string[]): string[] {
  return words.filter((w) => !BIP39_WORD_SET.has(w))
}

/// Verify the BIP39 checksum: the last (n / 32) bits of the entropy are the
/// first (n / 32) bits of SHA-256(entropy), where n is the entropy length in
/// bits.  Mnemonic length determines bit counts:
///   12 words → 128 bits entropy + 4 bit checksum (132 bits / 11 = 12)
///   24 words → 256 bits entropy + 8 bit checksum (264 bits / 11 = 24)
export async function checkMnemonic(input: string): Promise<MnemonicCheck> {
  const words = normalizeMnemonic(input)
  if (!VALID_COUNTS.has(words.length)) {
    return { kind: 'wrong_count', count: words.length }
  }
  const bad = checkWordlist(words)
  if (bad.length) return { kind: 'invalid_words', words: bad }

  // Each word maps to an 11-bit index. Pack them MSB-first into a bit string.
  const bits: number[] = []
  for (const w of words) {
    const idx = BIP39_WORDS.indexOf(w)
    for (let i = 10; i >= 0; i--) bits.push((idx >> i) & 1)
  }
  const totalBits = words.length * 11
  const entropyBits = (totalBits * 32) / 33      // 128, 160, 192, 224, 256
  const checksumBits = totalBits - entropyBits   // 4, 5, 6, 7, 8

  // Convert entropy bits to bytes for hashing.
  const entropy = new Uint8Array(entropyBits / 8)
  for (let i = 0; i < entropyBits; i++) {
    entropy[i >> 3] |= bits[i] << (7 - (i & 7))
  }

  const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', entropy))
  // The checksum is the first `checksumBits` bits of the hash.
  for (let i = 0; i < checksumBits; i++) {
    const expected = (hash[i >> 3] >> (7 - (i & 7))) & 1
    if (bits[entropyBits + i] !== expected) return { kind: 'bad_checksum' }
  }
  return { kind: 'ok' }
}
