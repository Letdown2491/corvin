import { describe, it, expect } from 'vitest'
import { parseImportFile, parseDescriptor, detectPaste, parseMultisigDescriptor } from './import'

// A structurally-valid xpub (the parser regex-matches shape, not base58 checksum).
const XPUB = 'xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz'
const FP = 'd34db33f'

describe('parseDescriptor', () => {
  it('parses wpkh with origin', () => {
    const r = parseDescriptor(`wpkh([${FP}/84'/0'/0']${XPUB}/0/*)`)
    expect(r).toEqual({
      xpub: XPUB,
      fingerprint: FP,
      path: "m/84'/0'/0'",
      accountType: 'native_segwit',
    })
  })

  it('parses sh(wpkh(...)) as p2sh_segwit', () => {
    const r = parseDescriptor(`sh(wpkh([${FP}/49'/0'/0']${XPUB}/0/*))`)
    expect(r?.accountType).toBe('p2sh_segwit')
    expect(r?.xpub).toBe(XPUB)
  })

  it('parses tr(...) as taproot', () => {
    expect(parseDescriptor(`tr(${XPUB}/0/*)`)?.accountType).toBe('taproot')
  })

  it('parses pkh(...) as legacy', () => {
    expect(parseDescriptor(`pkh(${XPUB}/0/*)`)?.accountType).toBe('legacy')
  })

  it('strips a checksum', () => {
    expect(parseDescriptor(`wpkh(${XPUB}/0/*)#abcd1234`)?.xpub).toBe(XPUB)
  })

  it('handles the multipath <0;1> variant', () => {
    expect(parseDescriptor(`wpkh([${FP}/84'/0'/0']${XPUB}/<0;1>/*)`)?.xpub).toBe(XPUB)
  })

  it('returns null for an unknown script type', () => {
    expect(parseDescriptor(`combo(${XPUB})`)).toBeNull()
  })

  it('bare xpub with no origin has no fingerprint/path', () => {
    const r = parseDescriptor(`wpkh(${XPUB}/0/*)`)
    expect(r?.xpub).toBe(XPUB)
    expect(r?.fingerprint).toBeUndefined()
  })
})

describe('parseImportFile', () => {
  it('parses a bare descriptor line', () => {
    expect(parseImportFile(`wpkh([${FP}/84'/0'/0']${XPUB}/0/*)`)?.xpub).toBe(XPUB)
  })

  it('parses Sparrow/Specter { descriptor }', () => {
    const json = JSON.stringify({ descriptor: `wpkh([${FP}/84'/0'/0']${XPUB}/<0;1>/*)` })
    const r = parseImportFile(json)
    expect(r?.xpub).toBe(XPUB)
    expect(r?.fingerprint).toBe(FP)
  })

  it('parses Coldcard generic JSON (p2wpkh + xfp)', () => {
    const json = JSON.stringify({
      xfp: FP.toUpperCase(),
      p2wpkh: { xpub: XPUB, deriv: "m/84'/0'/0'", name: 'p2wpkh' },
    })
    const r = parseImportFile(json)
    expect(r).toEqual({
      xpub: XPUB,
      fingerprint: FP,
      path: "m/84'/0'/0'",
      accountType: 'native_segwit',
    })
  })

  it('parses older Coldcard ExtPubKey field', () => {
    const json = JSON.stringify({ p2tr: { ExtPubKey: XPUB, deriv: "m/86'/0'/0'" } })
    const r = parseImportFile(json)
    expect(r?.xpub).toBe(XPUB)
    expect(r?.accountType).toBe('taproot')
  })

  it('parses a generic top-level { xpub }', () => {
    expect(parseImportFile(JSON.stringify({ xpub: XPUB }))?.xpub).toBe(XPUB)
  })

  it('regex fallback grabs an ext-key from loose text', () => {
    expect(parseImportFile(`my key is ${XPUB} ok`)?.xpub).toBe(XPUB)
  })

  it('returns null when there is no key', () => {
    expect(parseImportFile('not a wallet file')).toBeNull()
    expect(parseImportFile('{}')).toBeNull()
  })
})

describe('detectPaste', () => {
  it('classifies empty / whitespace', () => {
    expect(detectPaste('').kind).toBe('empty')
    expect(detectPaste('   ').kind).toBe('empty')
  })
  it('classifies bech32 + legacy addresses as address', () => {
    expect(detectPaste('bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4').kind).toBe('address')
    expect(detectPaste('tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxxxxxx').kind).toBe('address')
    expect(detectPaste('1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2').kind).toBe('address')
    expect(detectPaste('3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy').kind).toBe('address')
  })
  it('classifies xpub/ypub/zpub with the right type', () => {
    expect(detectPaste('zpubAbc123')).toMatchObject({ kind: 'xpub', type: 'zpub' })
    expect(detectPaste('ypubAbc123')).toMatchObject({ kind: 'xpub', type: 'ypub' })
    expect(detectPaste('xpubAbc123')).toMatchObject({ kind: 'xpub', type: 'xpub' })
    // testnet variants map to the same script type
    expect(detectPaste('vpubAbc')).toMatchObject({ kind: 'xpub', type: 'zpub' })
    expect(detectPaste('upubAbc')).toMatchObject({ kind: 'xpub', type: 'ypub' })
    expect(detectPaste('tpubAbc')).toMatchObject({ kind: 'xpub', type: 'xpub' })
  })
  it('does not misread a long xpub starting with 1-ish base58 as an address', () => {
    // xpubs are ~111 chars — well past the 62-char address ceiling
    expect(detectPaste('x'.repeat(80)).kind).toBe('unknown')
  })
  it('classifies anything else as unknown', () => {
    expect(detectPaste('hello world').kind).toBe('unknown')
  })
})

describe('parseMultisigDescriptor', () => {
  const A = `[aaaaaaaa/48h/0h/0h/2h]${XPUB}/0/*`
  const B = `[bbbbbbbb/48h/0h/0h/2h]${XPUB}/0/*`
  const C = `[cccccccc/48h/0h/0h/2h]${XPUB}/0/*`

  it('parses a 2-of-3 sortedmulti with origins + checksum', () => {
    const r = parseMultisigDescriptor(`wsh(sortedmulti(2,${A},${B},${C}))#checksum`)
    expect(r).not.toBeNull()
    expect(r?.threshold).toBe(2)
    expect(r?.signers).toHaveLength(3)
    expect(r?.signers[0].fingerprint).toBe('aaaaaaaa')
    expect(r?.signers[0].path).toBe('m/48h/0h/0h/2h')
    expect(r?.signers[0].xpub).toBe(XPUB)
  })
  it('accepts bare multi()', () => {
    expect(parseMultisigDescriptor(`wsh(multi(1,${A},${B}))`)?.threshold).toBe(1)
  })
  it('rejects non-wsh / missing origins / bad threshold', () => {
    expect(parseMultisigDescriptor(`wpkh(${XPUB})`)).toBeNull()
    expect(parseMultisigDescriptor(`wsh(sortedmulti(2,${XPUB},${XPUB}))`)).toBeNull() // no origins
    expect(parseMultisigDescriptor(`wsh(sortedmulti(5,${A},${B}))`)).toBeNull() // threshold > signers
    expect(parseMultisigDescriptor('garbage')).toBeNull()
  })
})
