// Shared QR decode/encode helpers. The scan side (QrScanModal, SendModal's
// address scan) and the display side (QrSignFlow) both lean on these so the
// BBQr / UR (crypto-psbt) handling lives in one place.

import { bytesToBase64 } from './utils'

// bbqr + @ngraveio/bc-ur are loaded on demand via QrCollector.init() so they
// stay out of the eager bundle. These are *type-only* imports (erased at build),
// used to type the cached references; the actual modules arrive via dynamic
// import() inside init().
type JoinQRs = typeof import('bbqr')['joinQRs']
type URDecoderCtor = typeof import('@ngraveio/bc-ur')['URDecoder']

/// Encode raw bytes as a single CBOR byte string. Inline because we only need
/// one type and pulling in a full CBOR lib would more than double the bundle
/// for this tiny use case.
export function encodeCborByteString(data: Uint8Array): Uint8Array {
  const n = data.length
  let header: number[]
  if (n < 24) header = [0x40 | n]
  else if (n < 0x100) header = [0x58, n]
  else if (n < 0x10000) header = [0x59, (n >> 8) & 0xff, n & 0xff]
  else header = [0x5a, (n >> 24) & 0xff, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff]
  const out = new Uint8Array(header.length + n)
  out.set(header, 0)
  out.set(data, header.length)
  return out
}

/// A PSBT carried in a QR is at most a few KB; cap well above that so a
/// malformed/hostile frame declaring a giant length is rejected outright rather
/// than driving a huge allocation.
const MAX_CBOR_PSBT_BYTES = 4 * 1024 * 1024

/// Reverse of `encodeCborByteString`. Tolerates either a bare byte string or
/// the same wrapped in a leading tag (some encoders use a CBOR tag). Input is
/// untrusted (scanned from a QR), so every length read is bounds-checked, the
/// 32-bit length is read unsigned, and an absurd length is refused.
export function decodeCborByteString(buf: Uint8Array): Uint8Array | null {
  let i = 0
  // Skip a single tag header if present.
  if (buf.length > 0 && (buf[i] & 0xe0) === 0xc0) {
    const minor = buf[i] & 0x1f
    i++
    if (minor === 24) i += 1
    else if (minor === 25) i += 2
    else if (minor === 26) i += 4
    else if (minor === 27) i += 8
  }
  if (i >= buf.length) return null
  const major = buf[i] >> 5
  if (major !== 2) return null
  const minor = buf[i] & 0x1f
  i++
  let len: number
  if (minor < 24) len = minor
  else if (minor === 24) { if (i + 1 > buf.length) return null; len = buf[i]; i += 1 }
  else if (minor === 25) { if (i + 2 > buf.length) return null; len = (buf[i] << 8) | buf[i + 1]; i += 2 }
  else if (minor === 26) {
    if (i + 4 > buf.length) return null
    // `>>> 0` keeps the high-bit-set case unsigned (a plain `<<` is signed 32-bit).
    len = ((buf[i] << 24) | (buf[i + 1] << 16) | (buf[i + 2] << 8) | buf[i + 3]) >>> 0
    i += 4
  } else return null
  if (len > MAX_CBOR_PSBT_BYTES) return null
  if (i + len > buf.length) return null
  return buf.slice(i, i + len)
}

/// Outcome of feeding one scanned frame into a `QrCollector`.
export type ScanOutcome =
  /// A multipart payload finished assembling into a PSBT (base64).
  | { kind: 'psbt'; base64: string }
  /// A single, non-multipart QR — its raw text (address, BIP21 URI, bare PSBT…).
  | { kind: 'text'; value: string }
  /// More frames needed; `have`/`need` drive the progress UI.
  | { kind: 'progress'; have: number; need: number }
  /// Recoverable: duplicate frame or a misread we should keep scanning past.
  | { kind: 'pending' }
  /// Unrecoverable decode failure for the current stream.
  | { kind: 'error'; message: string }

/// Accumulates scanned QR frames. Single-frame codes resolve immediately as
/// `text`; BBQr (`B$…`) and UR (`ur:…`, fountain-coded) frames accumulate until
/// the payload reconstructs into a PSBT. One instance per scan session.
export class QrCollector {
  private bbqrParts = new Set<string>()
  private bbqrNeed = 0
  private urDecoder: InstanceType<URDecoderCtor> | null = null
  private joinQRs: JoinQRs | null = null
  private URDecoder: URDecoderCtor | null = null

  /// Preload the multipart codecs (bbqr + bc-ur). Await once before feeding
  /// frames to `ingest()`, which then uses the cached refs synchronously.
  async init() {
    if (!this.joinQRs) this.joinQRs = (await import('bbqr')).joinQRs
    if (!this.URDecoder) this.URDecoder = (await import('@ngraveio/bc-ur')).URDecoder
  }

  reset() {
    this.bbqrParts.clear()
    this.bbqrNeed = 0
    this.urDecoder = null
  }

  ingest(data: string): ScanOutcome {
    const lower = data.toLowerCase()

    // BBQr: header is "B$" + encoding(1) + filetype(1) + total(base36,2) +
    // index(base36,2) = 8 chars.
    if (data.startsWith('B$')) {
      if (this.bbqrParts.has(data)) return { kind: 'pending' }
      this.bbqrParts.add(data)
      if (data.length >= 8) {
        const n = parseInt(data.slice(4, 6), 36)
        if (!isNaN(n) && n > 0) this.bbqrNeed = n
      }
      const have = this.bbqrParts.size
      if (this.bbqrNeed > 0 && have >= this.bbqrNeed && this.joinQRs) {
        try {
          const { raw } = this.joinQRs([...this.bbqrParts])
          return { kind: 'psbt', base64: bytesToBase64(raw) }
        } catch {
          // Header said complete but join rejected — a frame was likely
          // misread. Keep scanning.
          return { kind: 'pending' }
        }
      }
      return { kind: 'progress', have, need: this.bbqrNeed }
    }

    // UR (crypto-psbt): fountain-coded, frame N+1 may be a parity mix of prior
    // frames. Hold one decoder and feed it until it has enough parity.
    if (lower.startsWith('ur:')) {
      if (!this.URDecoder) return { kind: 'pending' }
      try {
        if (!this.urDecoder) this.urDecoder = new this.URDecoder()
        this.urDecoder.receivePart(data)
        if (this.urDecoder.isComplete() && this.urDecoder.isSuccess()) {
          const ur = this.urDecoder.resultUR()
          const psbtBytes = decodeCborByteString(new Uint8Array(ur.cbor))
          if (psbtBytes) return { kind: 'psbt', base64: bytesToBase64(psbtBytes) }
          return { kind: 'error', message: 'Decoded UR but its payload was not a recognisable PSBT.' }
        }
        if (this.urDecoder.isError()) {
          const message = `UR decode error: ${this.urDecoder.resultError()}`
          this.urDecoder = null
          return { kind: 'error', message }
        }
        return {
          kind: 'progress',
          have: this.urDecoder.receivedPartIndexes().length,
          need: this.urDecoder.expectedPartCount(),
        }
      } catch {
        // Tolerate occasional misreads — the fountain decoder is resilient and
        // re-accepts duplicates safely.
        return { kind: 'pending' }
      }
    }

    // Anything else is a single, self-contained QR.
    return { kind: 'text', value: data }
  }
}
