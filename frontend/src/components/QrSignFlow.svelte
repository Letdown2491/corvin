<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let QRCode: any = null
  async function getQRCode() {
    if (!QRCode) QRCode = await import('qrcode')
    return QRCode
  }
  import { base64ToBytes, bytesToBase64 } from '../lib/utils'

  // bbqr + bc-ur + buffer load on demand (only the display/encode path uses
  // them), keeping ~357 kB out of the eager bundle. loadEncoders() is awaited
  // before the first encode in init().
  let bbqr: typeof import('bbqr') | null = null
  let bcur: typeof import('@ngraveio/bc-ur') | null = null
  let BufferCtor: typeof import('buffer')['Buffer'] | null = null
  async function loadEncoders() {
    if (!bbqr) bbqr = await import('bbqr')
    if (!bcur) bcur = await import('@ngraveio/bc-ur')
    if (!BufferCtor) BufferCtor = (await import('buffer')).Buffer
  }
  import { encodeCborByteString, QrCollector } from '../lib/qr'

  // jsQR (~257 KB) is loaded on demand when scanning starts, kept out of the
  // eager bundle. Cached so the per-frame decode below stays synchronous.
  let jsQR: typeof import('jsqr').default | null = null

  let {
    psbt,
    onSigned,
    onCancel,
  }: {
    psbt: string
    onSigned: (psbt: string) => void | Promise<void>
    onCancel: () => void
  } = $props()

  /// Display-side QR format. BBQr is Coldcard / Passport / Krux / Foundation
  /// Core / SeedSigner (in BBQr mode). UR (`crypto-psbt`) is Keystone /
  /// Blockstream Jade / SeedSigner (in UR mode). The scanner accepts either
  /// format regardless of which the display is showing.
  type QrFormat = 'bbqr' | 'ur'
  let format = $state<QrFormat>('bbqr')

  // ── Display state ─────────────────────────────────────────────────────────
  let parts = $state<string[]>([])
  let frameIdx = $state(0)
  let qrDataUrl = $state('')
  let animTimer: ReturnType<typeof setInterval> | null = null

  // ── Scan state ────────────────────────────────────────────────────────────
  let scanning = $state(false)
  let videoEl: HTMLVideoElement | null = $state(null)
  let canvasEl: HTMLCanvasElement | null = $state(null)
  let mediaStream: MediaStream | null = null
  let rafId: number | null = null
  let scanError = $state('')
  let partsHave = $state(0)
  let partsNeed = $state(0)
  /// Accumulates scanned frames (BBQr + UR fountain) until a PSBT reconstructs.
  const collector = new QrCollector()

  // ── No-camera fallback ──────────────────────────────────────────────────────
  // Same signed-PSBT return path as the scanner (`onSigned`), for when there's no
  // webcam: load the signed `.psbt`/`.txt` the device wrote, or paste it.
  let pasteText = $state('')
  let importError = $state('')

  function fileToPsbtBase64(bytes: Uint8Array): string {
    // Binary PSBT magic = 70 73 62 74 ff; otherwise the bytes are base64 text.
    const isBinary =
      bytes.length >= 5 &&
      bytes[0] === 0x70 && bytes[1] === 0x73 && bytes[2] === 0x62 && bytes[3] === 0x74 && bytes[4] === 0xff
    return isBinary
      ? bytesToBase64(bytes)
      : new TextDecoder().decode(bytes).trim().replace(/\s+/g, '')
  }

  async function importSignedFile(e: Event) {
    const input = e.target as HTMLInputElement
    const file = input.files?.[0]
    input.value = '' // allow re-picking the same file
    if (!file) return
    importError = ''
    try {
      const b64 = fileToPsbtBase64(new Uint8Array(await file.arrayBuffer()))
      if (!b64) throw new Error('the file is empty')
      await onSigned(b64)
    } catch (err) {
      importError = err instanceof Error ? err.message : 'Could not read the PSBT file'
    }
  }

  async function importPastedPsbt() {
    const b64 = pasteText.trim().replace(/\s+/g, '')
    if (!b64) return
    importError = ''
    try {
      await onSigned(b64)
    } catch (err) {
      importError = err instanceof Error ? err.message : 'Could not import the PSBT'
    }
  }

  // ── Helpers ───────────────────────────────────────────────────────────────
  // Shared with SendModal etc. — see lib/utils.ts.

  // UR uses a fountain encoder (frame N+1 may be a parity-encoded mix of
  // previously-shown frames, not a fixed sequential chunk). We hold the
  // encoder across renders and pull `nextPart()` each tick.
  let urEncoder: import('@ngraveio/bc-ur').UREncoder | null = null

  // ── Display ───────────────────────────────────────────────────────────────
  async function init() {
    await loadEncoders()
    const raw = base64ToBytes(psbt)
    if (format === 'bbqr') {
      urEncoder = null
      const result = bbqr!.splitQRs(raw, 'P')
      parts = result.parts
      frameIdx = 0
      await renderFrame(0)
    } else {
      // crypto-psbt UR type: the CBOR payload is a byte string of the PSBT
      // bytes. We build it by hand since we only need to encode one byte
      // string (CBOR major type 2). bytestring header + length prefix + data.
      const cbor = encodeCborByteString(raw)
      const ur = new bcur!.UR(BufferCtor!.from(cbor), 'crypto-psbt')
      // 200 bytes/fragment keeps each QR code well within the v15 / EC-L
      // capacity (~520 alnum chars after the ur: prefix), so most cameras
      // can scan a single frame without zooming.
      urEncoder = new bcur!.UREncoder(ur, 200, 0, 10)
      parts = []
      frameIdx = 0
      // Render an initial frame so something appears before the anim ticks.
      await renderUrFrame()
    }
    startAnim()
  }

  async function renderUrFrame() {
    if (!urEncoder) return
    const text = urEncoder.nextPart()
    try {
      const QR = await getQRCode()
      qrDataUrl = await QR.toDataURL(text, {
        errorCorrectionLevel: 'L',
        width: 240,
        margin: 2,
        color: { dark: '#000000', light: '#ffffff' },
      })
    } catch {}
  }

  async function renderFrame(idx: number) {
    if (!parts.length) return
    try {
      const QR = await getQRCode()
      qrDataUrl = await QR.toDataURL(parts[idx % parts.length], {
        errorCorrectionLevel: 'L',
        width: 240,
        margin: 2,
        color: { dark: '#000000', light: '#ffffff' },
      })
    } catch {}
  }

  function startAnim() {
    stopAnim()
    if (format === 'ur' && urEncoder) {
      // UR fountain encoder advances on every tick — no notion of "all
      // frames shown once," the receiver decides when it has enough.
      animTimer = setInterval(async () => {
        frameIdx = (frameIdx + 1) % Math.max(1, urEncoder!.fragmentsLength)
        await renderUrFrame()
      }, 250)
      return
    }
    if (parts.length <= 1) return
    animTimer = setInterval(async () => {
      frameIdx = (frameIdx + 1) % parts.length
      await renderFrame(frameIdx)
    }, 250)
  }

  function stopAnim() {
    if (animTimer !== null) { clearInterval(animTimer); animTimer = null }
  }

  // ── Scanner ───────────────────────────────────────────────────────────────
  async function startScan() {
    scanning = true
    scanError = ''
    collector.reset()
    partsHave = 0
    partsNeed = 0

    try {
      if (!jsQR) jsQR = (await import('jsqr')).default
      await collector.init()
      mediaStream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: 'environment' },
      })
      if (videoEl) {
        videoEl.srcObject = mediaStream
        await videoEl.play()
      }
      tick()
    } catch (e) {
      scanError = e instanceof Error ? e.message : 'Camera access denied'
      scanning = false
    }
  }

  function tick() {
    if (!scanning || !videoEl || !canvasEl) return
    if (!jsQR || videoEl.readyState < 2 || videoEl.videoWidth === 0) {
      rafId = requestAnimationFrame(tick)
      return
    }

    const ctx = canvasEl.getContext('2d')!
    canvasEl.width = videoEl.videoWidth
    canvasEl.height = videoEl.videoHeight
    ctx.drawImage(videoEl, 0, 0)

    const img = ctx.getImageData(0, 0, canvasEl.width, canvasEl.height)
    const found = jsQR(img.data, img.width, img.height)

    // Auto-detect format inside the collector: BBQr frames start with `B$`,
    // UR frames with `ur:`. Either flavour can arrive regardless of which we're
    // displaying — air-gapped signers don't necessarily echo the input format.
    if (found?.data) {
      const outcome = collector.ingest(found.data.trim())
      if (outcome.kind === 'psbt') {
        stopScanStream()
        onSigned(outcome.base64)
        return
      } else if (outcome.kind === 'progress') {
        partsHave = outcome.have
        partsNeed = outcome.need
      } else if (outcome.kind === 'error') {
        scanError = outcome.message
      }
      // 'text' (a non-PSBT QR) and 'pending' (dup/misread) → keep scanning.
    }

    rafId = requestAnimationFrame(tick)
  }

  function stopScanStream() {
    if (rafId !== null) { cancelAnimationFrame(rafId); rafId = null }
    if (mediaStream) { mediaStream.getTracks().forEach(t => t.stop()); mediaStream = null }
    if (videoEl) videoEl.srcObject = null
    scanning = false
  }

  function backToDisplay() {
    stopScanStream()
    startAnim()
  }

  onMount(init)
  onDestroy(() => { stopAnim(); stopScanStream() })
</script>

<div class="qr-flow">
  {#if !scanning}
    <p class="qr-hint">Scan with your hardware wallet, then tap below to read the signed result.</p>
    <div class="qr-format-toggle" role="group" aria-label="QR format">
      <button
        type="button"
        class:active={format === 'bbqr'}
        onclick={async () => { format = 'bbqr'; await init() }}
        title="BBQr — Coldcard, Passport, Krux, Foundation, SeedSigner (default)"
      >BBQr</button>
      <button
        type="button"
        class:active={format === 'ur'}
        onclick={async () => { format = 'ur'; await init() }}
        title="UR (crypto-psbt) — Keystone, Blockstream Jade, SeedSigner UR mode"
      >UR</button>
    </div>
    <div class="qr-wrap">
      {#if qrDataUrl}
        <img src={qrDataUrl} alt="PSBT QR code" class="qr-img" />
      {:else}
        <div class="qr-placeholder">Generating…</div>
      {/if}
      {#if format === 'bbqr' && parts.length > 1}
        <p class="qr-frame-info">Frame {frameIdx + 1} of {parts.length}</p>
      {:else if format === 'ur'}
        <p class="qr-frame-info">UR frame {frameIdx + 1} (fountain-encoded)</p>
      {/if}
    </div>
    <div class="qr-actions">
      <button class="btn-scan-qr" onclick={startScan}>Scan signed result →</button>
      <button class="btn-cancel-qr" onclick={onCancel}>Cancel</button>
    </div>
    <details class="qr-nocam">
      <summary>No camera? Import the signed PSBT instead</summary>
      <div class="qr-nocam-body">
        <label class="qr-file-btn" title="Load the signed .psbt the device wrote">
          ↓ Load signed file
          <input type="file" accept=".psbt,.txt" onchange={importSignedFile} />
        </label>
        <textarea
          class="qr-paste"
          rows="2"
          placeholder="Or paste the signed base64 PSBT here"
          spellcheck="false"
          bind:value={pasteText}
        ></textarea>
        {#if importError}<p class="qr-import-err">{importError}</p>{/if}
        <button class="btn-import-psbt" disabled={!pasteText.trim()} onclick={importPastedPsbt}>
          Import pasted PSBT
        </button>
      </div>
    </details>
  {:else}
    <p class="qr-hint">Point camera at the QR code displayed by your hardware wallet.</p>
    <div class="video-wrap">
      <video bind:this={videoEl} class="qr-video" muted playsinline></video>
      <canvas bind:this={canvasEl} class="qr-canvas"></canvas>
      <div class="scan-reticle"></div>
    </div>
    {#if partsNeed > 1}
      <p class="scan-progress">{partsHave} of {partsNeed} frames received</p>
    {:else}
      <p class="scan-progress">Scanning…</p>
    {/if}
    {#if scanError}
      <p class="scan-error">{scanError}</p>
    {/if}
    <div class="qr-actions">
      <button class="btn-cancel-qr" onclick={backToDisplay}>← Back</button>
      <button class="btn-cancel-qr" onclick={onCancel}>Cancel</button>
    </div>
  {/if}
</div>

<style>
  .qr-flow {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  .qr-hint {
    font-size: 0.78rem;
    color: var(--text-muted);
    margin: 0;
    line-height: 1.4;
  }

  .qr-wrap {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .qr-img {
    display: block;
    width: 220px;
    height: 220px;
    border-radius: 6px;
    background: #fff;
  }

  .qr-placeholder {
    width: 220px;
    height: 220px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-2);
    border-radius: 6px;
    font-size: 0.78rem;
    color: var(--text-muted);
  }

  .qr-frame-info {
    font-size: 0.72rem;
    color: var(--text-muted);
    margin: 0;
  }

  .qr-format-toggle {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
    margin-bottom: 6px;
  }
  .qr-format-toggle button {
    background: var(--surface-2);
    border: none;
    color: var(--text-muted);
    padding: 4px 10px;
    cursor: pointer;
    font-size: 0.72rem;
    font-weight: 600;
  }
  .qr-format-toggle button + button { border-left: 1px solid var(--border); }
  .qr-format-toggle button.active { background: var(--accent); color: #000; }
  .qr-format-toggle button:hover:not(.active) { color: var(--text); }

  .video-wrap {
    position: relative;
    width: 100%;
    max-height: 220px;
    overflow: hidden;
    border-radius: 6px;
    background: #000;
  }

  .qr-video {
    display: block;
    width: 100%;
    max-height: 220px;
    object-fit: cover;
  }

  .qr-canvas {
    display: none;
  }

  .scan-reticle {
    position: absolute;
    inset: 0;
    border: 2px solid var(--accent);
    border-radius: 6px;
    pointer-events: none;
    animation: pulse 1.5s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 0.4; }
    50% { opacity: 1; }
  }

  .scan-progress {
    font-size: 0.78rem;
    color: var(--text-muted);
    margin: 0;
    text-align: center;
  }

  .scan-error {
    font-size: 0.78rem;
    color: #e57373;
    margin: 0;
  }

  .qr-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }

  .btn-scan-qr {
    flex: 1;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    border: 1px solid var(--accent);
    border-radius: 6px;
    padding: 7px 14px;
    font-size: 0.8rem;
    color: var(--accent);
    cursor: pointer;
    transition: all 0.12s;
  }
  .btn-scan-qr:hover { background: color-mix(in srgb, var(--accent) 20%, var(--surface-2)); }

  .btn-cancel-qr {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 7px 14px;
    font-size: 0.8rem;
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.12s;
  }
  .btn-cancel-qr:hover { border-color: var(--text-muted); color: var(--text); }

  .qr-nocam { margin-top: 4px; font-size: 0.8rem; }
  .qr-nocam summary {
    cursor: pointer; color: var(--text-muted); padding: 4px 0; user-select: none;
  }
  .qr-nocam summary:hover { color: var(--text); }
  .qr-nocam-body { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; }
  .qr-file-btn {
    align-self: flex-start; cursor: pointer;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 6px 12px; font-size: 0.8rem; color: var(--text-muted);
  }
  .qr-file-btn:hover { border-color: var(--text-muted); color: var(--text); }
  .qr-file-btn input[type="file"] { display: none; }
  .qr-paste {
    width: 100%; box-sizing: border-box; resize: vertical;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    color: var(--text); padding: 7px 10px; font-size: 0.78rem; font-family: var(--font-mono, monospace);
  }
  .qr-paste:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .qr-import-err { margin: 0; color: #e05a4f; font-size: 0.75rem; }
  .btn-import-psbt {
    align-self: flex-start;
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    border: 1px solid var(--accent); border-radius: 6px;
    padding: 6px 14px; font-size: 0.8rem; color: var(--accent); cursor: pointer;
  }
  .btn-import-psbt:disabled { opacity: 0.5; cursor: default; }
</style>
