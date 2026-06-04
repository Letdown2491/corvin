// Wallet-import file parsing, shared by the add-wallet flow and the vault
// signer cards. Pulls an xpub (and origin info, when present) out of the
// common export formats: Coldcard generic JSON, Sparrow/Specter descriptor
// JSON, a bare descriptor string, or a loose xpub.

export interface ParsedImport {
  xpub: string
  fingerprint?: string
  path?: string
  accountType?: 'native_segwit' | 'p2sh_segwit' | 'legacy' | 'taproot'
}

/// Tries (in order):
///   1. Coldcard generic JSON: `{ "p2wpkh": { "xpub": "...", "deriv": "m/84'/0'/0'", "name": "p2wpkh" }, ... , "xfp": "..." }`
///   2. Coldcard generic with `ExtPubKey` (older format)
///   3. Sparrow / Specter descriptor JSON: `{ "descriptor": "wpkh([fp/path]xpub.../<0;1>/*)" }`
///   4. Plain descriptor string in file body
///   5. Top-level `{ "xpub": "..." }` etc.
///   6. Loose regex fallback (last resort)
export function parseImportFile(text: string): ParsedImport | null {
  const trimmed = text.trim()

  // (3 / 4) Plain descriptor on a single line
  const descFromBody = trimmed.match(/^(?:wpkh|sh|pkh|tr|wsh)\(.*\)(?:#[a-z0-9]+)?$/i)
  if (descFromBody) {
    const fromDesc = parseDescriptor(descFromBody[0])
    if (fromDesc) return fromDesc
  }

  try {
    const obj = JSON.parse(trimmed)

    // (3) Sparrow/Specter: top-level "descriptor" field
    if (typeof obj.descriptor === 'string') {
      const fromDesc = parseDescriptor(obj.descriptor)
      if (fromDesc) return fromDesc
    }

    // (1) Coldcard generic export
    const xfp: string | undefined = (obj.xfp || obj.fingerprint || obj.master_fingerprint)?.toString().toLowerCase()
    const ctMap: Record<string, ParsedImport['accountType']> = {
      p2wpkh: 'native_segwit',
      'p2wpkh-p2sh': 'p2sh_segwit',
      'p2sh-p2wpkh': 'p2sh_segwit',
      p2pkh: 'legacy',
      p2tr: 'taproot',
    }
    for (const [key, accountType] of Object.entries(ctMap)) {
      const s = obj[key]
      if (s) {
        const xpub: string | undefined = s.xpub ?? s.ExtPubKey
        const path: string | undefined = s.deriv ?? s.derivation ?? s.path
        if (xpub) return { xpub, fingerprint: xfp, path, accountType }
      }
    }

    // (5) Generic top-level xpub fields
    const direct = obj.xpub ?? obj.ExtPubKey ?? obj.bip84_xpub ?? obj.bip49_xpub ?? obj.bip44_xpub ?? obj.bip86_xpub
    if (direct) {
      const path = obj.deriv ?? obj.derivation ?? obj.path
      return { xpub: direct, fingerprint: xfp, path }
    }
  } catch {}

  // (6) Regex fallback — grab the first ext-key in the file
  const match = trimmed.match(/[xyz]pub[A-Za-z0-9]{100,}|[tvu]pub[A-Za-z0-9]{100,}/)
  return match ? { xpub: match[0] } : null
}

/// Parse `wpkh([fingerprint/path]xpub/0/*)` (and the multipath
/// `<0;1>` variant) — extract xpub, fingerprint, derivation path, and
/// script type.
export function parseDescriptor(desc: string): ParsedImport | null {
  // Strip checksum
  const noSum = desc.replace(/#[a-z0-9]+$/i, '').trim()

  // Outer script type
  let accountType: ParsedImport['accountType']
  let inner = noSum
  const stripWrap = (re: RegExp) => {
    const m = inner.match(re)
    if (m) inner = m[1]
  }
  if (noSum.startsWith('wpkh(')) { accountType = 'native_segwit'; stripWrap(/^wpkh\((.*)\)$/) }
  else if (noSum.startsWith('sh(wpkh(')) { accountType = 'p2sh_segwit'; stripWrap(/^sh\(wpkh\((.*)\)\)$/) }
  else if (noSum.startsWith('pkh(')) { accountType = 'legacy'; stripWrap(/^pkh\((.*)\)$/) }
  else if (noSum.startsWith('tr(')) { accountType = 'taproot'; stripWrap(/^tr\((.*)\)$/) }
  else return null

  // [fingerprint/path]xpub/...
  const origin = inner.match(/^\[([0-9a-fA-F]{8})\/([^\]]+)\]([xyztuvXYZTUV]pub[A-Za-z0-9]+)/)
  if (origin) {
    return {
      xpub: origin[3],
      fingerprint: origin[1].toLowerCase(),
      path: 'm/' + origin[2],
      accountType,
    }
  }

  // No origin — just the bare xpub
  const bare = inner.match(/([xyztuvXYZTUV]pub[A-Za-z0-9]+)/)
  if (bare) return { xpub: bare[1], accountType }
  return null
}

/// Classification of a string pasted into the add-wallet input, used to show a
/// live hint and route to the right flow. A client-side heuristic — the backend
/// validates fully.
export type PasteKind =
  | { kind: 'address'; label: string }
  | { kind: 'xpub'; label: string; type: 'xpub' | 'ypub' | 'zpub' }
  | { kind: 'unknown' }
  | { kind: 'empty' }

export function detectPaste(s: string): PasteKind {
  const t = s.trim().toLowerCase()
  if (!t) return { kind: 'empty' }
  // Bech32 / legacy address heuristics — covers segwit, taproot, P2PKH, P2SH on
  // all networks. Length check rejects pasted xpubs that happen to start with '1'.
  if (
    t.startsWith('bc1') ||
    t.startsWith('tb1') ||
    t.startsWith('bcrt1') ||
    (/^[1-9A-HJ-NP-Za-km-z]+$/.test(s.trim()) &&
      t.length >= 26 &&
      t.length <= 62 &&
      (t.startsWith('1') || t.startsWith('3')))
  ) {
    return { kind: 'address', label: 'Bitcoin address — watch-only' }
  }
  if (t.startsWith('zpub') || t.startsWith('vpub'))
    return { kind: 'xpub', type: 'zpub', label: 'zpub — Native SegWit HD wallet' }
  if (t.startsWith('ypub') || t.startsWith('upub'))
    return { kind: 'xpub', type: 'ypub', label: 'ypub — P2SH-SegWit HD wallet' }
  if (t.startsWith('xpub') || t.startsWith('tpub'))
    return { kind: 'xpub', type: 'xpub', label: 'xpub — Legacy HD wallet' }
  return { kind: 'unknown' }
}

export interface ParsedMsSigner {
  name: string
  fingerprint: string
  path: string
  xpub: string
  accountIndex: number
}

/// Parse a `wsh(sortedmulti(...))` / `wsh(multi(...))` descriptor into its
/// threshold + cosigners. Each key must carry a `[fingerprint/path]` origin tag.
/// Returns null on anything it can't confidently parse.
export function parseMultisigDescriptor(
  raw: string,
): { threshold: number; signers: ParsedMsSigner[] } | null {
  const desc = raw.trim().replace(/#[a-z0-9]+$/i, '')
  // Accept both sortedmulti and bare multi; sorted is the safer default.
  let inner: string
  if (desc.startsWith('wsh(sortedmulti(')) {
    const m = desc.match(/^wsh\(sortedmulti\((.*)\)\)$/)
    if (!m) return null
    inner = m[1]
  } else if (desc.startsWith('wsh(multi(')) {
    const m = desc.match(/^wsh\(multi\((.*)\)\)$/)
    if (!m) return null
    inner = m[1]
  } else {
    return null
  }
  const parts = inner.split(',').map((s) => s.trim())
  if (parts.length < 3) return null
  const threshold = parseInt(parts[0], 10)
  if (!Number.isFinite(threshold) || threshold < 1) return null

  const signers: ParsedMsSigner[] = []
  for (const key of parts.slice(1)) {
    // Expected shape: [fp/path]xpub/0/*  or  [fp/path]xpub/<0;1>/*
    const m = key.match(/^\[([0-9a-fA-F]{8})\/([^\]]+)\]([xyztuvXYZTUV]pub[A-Za-z0-9]+)(?:\/.*)?$/)
    if (!m) return null
    signers.push({
      name: '',
      fingerprint: m[1].toLowerCase(),
      path: 'm/' + m[2],
      xpub: m[3],
      accountIndex: 0,
    })
  }
  if (threshold > signers.length) return null
  return { threshold, signers }
}
