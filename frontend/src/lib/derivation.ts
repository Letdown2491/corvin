// Parses script type, fingerprint, and path out of a wallet's descriptor.

import type { WalletEntry } from './types'

export type ScriptType = { label: string; bip: string }
export type CosignerInfo = { fingerprint: string; path: string }
export type DerivationInfo = {
  scriptType: ScriptType
  threshold: number | null
  cosigners: CosignerInfo[]
}

const ORIGIN_RE = /\[([a-fA-F0-9]{8})((?:\/\d+'?)+)\]/g
const MULTI_RE = /(?:sortedmulti|multi)(?:_a)?\((\d+)/

export function parseOrigins(descriptor: string): CosignerInfo[] {
  const out: CosignerInfo[] = []
  let m: RegExpExecArray | null
  // Reset the regex's lastIndex since it's a global re-used across calls.
  ORIGIN_RE.lastIndex = 0
  while ((m = ORIGIN_RE.exec(descriptor)) !== null) {
    out.push({ fingerprint: m[1].toLowerCase(), path: 'm' + m[2] })
  }
  return out
}

export function scriptTypeFor(path: string, isMultisig: boolean): ScriptType | null {
  const m = path.match(/^m\/(\d+)'/)
  if (!m) return null
  const purpose = parseInt(m[1], 10)
  if (isMultisig) {
    if (purpose === 48) {
      // BIP-48 depth-4: 1'=nested segwit, 2'=native segwit, 3'=taproot.
      const sub = path.match(/^m\/48'\/\d+'\/\d+'\/(\d+)'/)
      if (sub) {
        const s = parseInt(sub[1], 10)
        if (s === 1) return { label: 'Nested SegWit Multisig', bip: 'BIP-48' }
        if (s === 2) return { label: 'Native SegWit Multisig', bip: 'BIP-48' }
        if (s === 3) return { label: 'Taproot Multisig', bip: 'BIP-48' }
      }
      return { label: 'Multisig', bip: 'BIP-48' }
    }
    if (purpose === 45) return { label: 'Legacy Multisig', bip: 'BIP-45' }
    if (purpose === 87) return { label: 'Multisig', bip: 'BIP-87' }
    return { label: 'Multisig', bip: `m/${purpose}'` }
  }
  switch (purpose) {
    case 44: return { label: 'Legacy', bip: 'BIP-44' }
    case 49: return { label: 'Nested SegWit', bip: 'BIP-49' }
    case 84: return { label: 'Native SegWit', bip: 'BIP-84' }
    case 86: return { label: 'Taproot', bip: 'BIP-86' }
  }
  return null
}

export function parseDerivation(wallet: WalletEntry): DerivationInfo | null {
  if (wallet.kind === 'address') return null
  const cosigners = parseOrigins(wallet.external_descriptor)
  if (cosigners.length === 0) return null
  const isMultisig = wallet.kind === 'multisig'
  const scriptType = scriptTypeFor(cosigners[0].path, isMultisig)
  if (!scriptType) return null
  let threshold: number | null = null
  if (isMultisig) {
    const m = wallet.external_descriptor.match(MULTI_RE)
    if (m) threshold = parseInt(m[1], 10)
  }
  return { scriptType, threshold, cosigners }
}
