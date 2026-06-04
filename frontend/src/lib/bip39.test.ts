import { describe, it, expect } from 'vitest'
import { normalizeMnemonic, checkWordlist, checkMnemonic } from './bip39'

describe('normalizeMnemonic', () => {
  it('lowercases, trims, and collapses whitespace', () => {
    expect(normalizeMnemonic('  Abandon   ABILITY\tablE\n')).toEqual(['abandon', 'ability', 'able'])
  })
  it('drops empty tokens', () => {
    expect(normalizeMnemonic('abandon   ability')).toEqual(['abandon', 'ability'])
  })
})

describe('checkWordlist', () => {
  it('returns words not in the BIP39 list', () => {
    expect(checkWordlist(['abandon', 'notaword', 'ability', 'zzzz'])).toEqual(['notaword', 'zzzz'])
  })
  it('returns empty when all words are valid', () => {
    expect(checkWordlist(['abandon', 'ability', 'able'])).toEqual([])
  })
})

describe('checkMnemonic', () => {
  it('accepts a known-valid 12-word mnemonic', async () => {
    // Canonical BIP39 all-zeros-entropy test vector (valid checksum).
    const m = 'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about'
    expect(await checkMnemonic(m)).toEqual({ kind: 'ok' })
  })

  it('accepts a known-valid 24-word mnemonic', async () => {
    const m = Array(23).fill('abandon').join(' ') + ' art'
    expect(await checkMnemonic(m)).toEqual({ kind: 'ok' })
  })

  it('flags wrong word count', async () => {
    const r = await checkMnemonic('abandon abandon abandon')
    expect(r).toEqual({ kind: 'wrong_count', count: 3 })
  })

  it('flags invalid words', async () => {
    const m = 'abandon '.repeat(11) + 'notaword'
    const r = await checkMnemonic(m.trim())
    expect(r.kind).toBe('invalid_words')
  })

  it('flags a bad checksum (valid words, wrong last word)', async () => {
    // 12 valid words but the checksum word is wrong (…abandon abandon).
    const m = Array(12).fill('abandon').join(' ')
    expect(await checkMnemonic(m)).toEqual({ kind: 'bad_checksum' })
  })
})
