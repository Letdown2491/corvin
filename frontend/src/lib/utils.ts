/**
 * Decode a base64 string into a Uint8Array. Used by PSBT export so the
 * resulting `.psbt` file is a real binary PSBT (which Coldcard, Sparrow, and
 * Electrum all expect when loading from SD card), not base64-in-a-.psbt
 * which some clients refuse.
 */
export function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

/** Inverse of base64ToBytes. */
export function bytesToBase64(bytes: Uint8Array): string {
  let bin = ''
  for (const b of bytes) bin += String.fromCharCode(b)
  return btoa(bin)
}

/**
 * Build a Blob containing a binary PSBT, suitable for downloading as a `.psbt`.
 */
export function psbtBlob(base64Psbt: string): Blob {
  // Cast through unknown to BlobPart — TS's Uint8Array<ArrayBufferLike> default
  // doesn't satisfy BlobPart's narrower ArrayBufferView<ArrayBuffer>, but the
  // runtime type is always a plain ArrayBuffer-backed Uint8Array here.
  const bytes = base64ToBytes(base64Psbt) as unknown as BlobPart
  return new Blob([bytes], { type: 'application/octet-stream' })
}

/**
 * In the desktop (Tauri) build, `withGlobalTauri` exposes
 * `window.__TAURI__.core.invoke`. Returns the invoke promise, or null in a
 * plain browser so callers can fall back to web behaviour.
 */
export function invokeDesktop<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T> | null {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const invoke = (window as any).__TAURI__?.core?.invoke
  return typeof invoke === 'function' ? (invoke(cmd, args) as Promise<T>) : null
}

/** True when running inside the desktop (Tauri) shell. */
export function isDesktop(): boolean {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  return typeof (window as any).__TAURI__?.core?.invoke === 'function'
}

/**
 * Downloads a Blob as a file. In the desktop shell the webview won't persist an
 * `<a download>` click, so route through a native Save dialog in the backend;
 * in a browser, use the anchor (deferring revokeObjectURL for Firefox timing).
 */
export function downloadBlob(blob: Blob, filename: string): void {
  if (isDesktop()) {
    blob.arrayBuffer().then((buf) =>
      invokeDesktop('save_file', { name: filename, contents: Array.from(new Uint8Array(buf)) })
        ?.catch((e) => console.error('save_file failed', e)),
    )
    return
  }
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  setTimeout(() => { a.remove(); URL.revokeObjectURL(url) }, 100)
}

/**
 * The one place the UTXO key format lives. Stores (labels, freezes) and the
 * UI key on this string; composing it inline elsewhere risks a silent lookup
 * miss if the format ever changes.
 */
export function utxoKey(txid: string, vout: number): string {
  return `${txid}:${vout}`
}

/**
 * Best-effort error sink — use instead of a bare `catch {}` so a swallowed
 * failure stays greppable and shows up in dev, without bothering the user.
 */
export function swallow(err: unknown, context?: string): void {
  if (import.meta.env.DEV) console.debug(`[swallowed]${context ? ` ${context}` : ''}`, err)
}

export function kindLabel(kind: string): string {
  switch (kind) {
    case 'address': return 'Watch address'
    case 'xpub': case 'ypub': case 'zpub': case 'taproot': return 'HD wallet'
    case 'multisig': return 'Multisig'
    case 'silent_payments': return 'Silent Payments'
    case 'descriptor': return 'Imported descriptor'
    default: return kind
  }
}
