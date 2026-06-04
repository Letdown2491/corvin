<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { QrCollector } from '../lib/qr'

  // jsQR (~257 KB) is loaded on demand the moment scanning starts, not in the
  // eager wallet bundle. Cached in a module var so the per-frame decode call
  // below stays synchronous once the camera is live.
  let jsQR: typeof import('jsqr').default | null = null

  let {
    title = 'Scan QR code',
    hint = 'Point your camera at the QR code.',
    validate,
    invalidHint = "That QR code wasn't what we expected. Keep scanning…",
    onResult,
    onCancel,
  }: {
    title?: string
    hint?: string
    /// Applied to single-frame results only. Return false to reject and keep
    /// scanning (multipart PSBT results are always accepted).
    validate?: (value: string) => boolean
    invalidHint?: string
    onResult: (value: string) => void
    onCancel: () => void
  } = $props()

  let videoEl = $state<HTMLVideoElement | null>(null)
  let canvasEl = $state<HTMLCanvasElement | null>(null)
  let stream: MediaStream | null = null
  let rafId: number | null = null
  let error = $state('')
  let progressHave = $state(0)
  let progressNeed = $state(0)
  let done = false
  const collector = new QrCollector()

  async function start() {
    error = ''
    if (!navigator.mediaDevices?.getUserMedia) {
      error = 'Camera is not available in this environment.'
      return
    }
    try {
      if (!jsQR) jsQR = (await import('jsqr')).default
      await collector.init()
      stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } })
      if (videoEl) { videoEl.srcObject = stream; await videoEl.play() }
      tick()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Camera access denied'
    }
  }

  function tick() {
    if (done || !videoEl || !canvasEl) return
    if (!jsQR || videoEl.readyState < 2 || videoEl.videoWidth === 0) {
      rafId = requestAnimationFrame(tick); return
    }
    const ctx = canvasEl.getContext('2d')!
    canvasEl.width = videoEl.videoWidth
    canvasEl.height = videoEl.videoHeight
    ctx.drawImage(videoEl, 0, 0)
    const img = ctx.getImageData(0, 0, canvasEl.width, canvasEl.height)
    const found = jsQR(img.data, img.width, img.height)
    if (found?.data) {
      const outcome = collector.ingest(found.data.trim())
      switch (outcome.kind) {
        case 'psbt':
          finish(outcome.base64); return
        case 'text':
          if (!validate || validate(outcome.value)) { finish(outcome.value); return }
          error = invalidHint
          break
        case 'progress':
          progressHave = outcome.have
          progressNeed = outcome.need
          error = ''
          break
        case 'error':
          error = outcome.message
          break
        case 'pending':
          break
      }
    }
    rafId = requestAnimationFrame(tick)
  }

  function finish(value: string) {
    done = true
    stop()
    onResult(value)
  }

  // No-camera fallback: paste the value the QR would have carried.
  let pasteVal = $state('')
  function submitPaste() {
    const v = pasteVal.trim()
    if (!v) return
    if (validate && !validate(v)) { error = "That doesn't look valid for this field."; return }
    finish(v)
  }

  function stop() {
    if (rafId !== null) { cancelAnimationFrame(rafId); rafId = null }
    if (stream) { stream.getTracks().forEach(t => t.stop()); stream = null }
    if (videoEl) videoEl.srcObject = null
  }

  function cancel() { stop(); onCancel() }

  onMount(start)
  onDestroy(stop)
</script>

<svelte:window onkeydown={(e) => { if (e.key === 'Escape') cancel() }} />

<div class="qr-scan-overlay" role="dialog" aria-modal="true" aria-label={title}>
  <div class="qr-scan-card">
    <h3 class="qr-scan-title">{title}</h3>
    <p class="qr-scan-hint">{hint}</p>
    <div class="qr-scan-video-wrap">
      <video bind:this={videoEl} class="qr-scan-video" muted playsinline></video>
      <canvas bind:this={canvasEl} style="display:none"></canvas>
      <div class="qr-scan-reticle"></div>
    </div>
    {#if progressNeed > 1}
      <p class="qr-scan-progress">{progressHave} of {progressNeed} frames received</p>
    {/if}
    {#if error}
      <p class="qr-scan-error">{error}</p>
    {/if}
    <details class="qr-scan-nocam">
      <summary>No camera? Paste instead</summary>
      <div class="qr-scan-nocam-body">
        <input
          class="qr-scan-paste"
          type="text"
          bind:value={pasteVal}
          placeholder="Paste the value here"
          spellcheck="false"
          autocapitalize="off"
          autocomplete="off"
          onkeydown={(e) => { if (e.key === 'Enter') submitPaste() }}
        />
        <button type="button" class="qr-scan-use" disabled={!pasteVal.trim()} onclick={submitPaste}>Use</button>
      </div>
    </details>
    <div class="qr-scan-actions">
      <button type="button" class="qr-scan-cancel" onclick={cancel}>Cancel</button>
    </div>
  </div>
</div>

<style>
  .qr-scan-overlay {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.6);
    padding: var(--space-4);
  }

  .qr-scan-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    max-width: 420px;
    padding: var(--space-4);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
  }

  .qr-scan-title {
    margin: 0;
    font-size: var(--text-lg);
    color: var(--text);
  }

  .qr-scan-hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    line-height: 1.4;
  }

  .qr-scan-video-wrap {
    position: relative;
    width: 100%;
    aspect-ratio: 1 / 1;
    overflow: hidden;
    border-radius: var(--radius);
    background: #000;
  }

  .qr-scan-video {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .qr-scan-reticle {
    position: absolute;
    inset: 18%;
    border: 2px solid var(--accent);
    border-radius: var(--radius);
    box-shadow: 0 0 0 100vmax rgba(0, 0, 0, 0.25);
    pointer-events: none;
  }

  .qr-scan-progress {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--accent);
    text-align: center;
  }

  .qr-scan-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--error);
    text-align: center;
  }

  .qr-scan-actions {
    display: flex;
    justify-content: flex-end;
  }

  .qr-scan-cancel {
    padding: var(--space-2) var(--space-4);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    color: var(--text);
    cursor: pointer;
  }

  .qr-scan-cancel:hover {
    background: var(--surface-hover);
  }
  .qr-scan-nocam { font-size: 0.82rem; color: var(--text-muted); }
  .qr-scan-nocam summary { cursor: pointer; padding: 4px 0; user-select: none; }
  .qr-scan-nocam summary:hover { color: var(--text); }
  .qr-scan-nocam-body { display: flex; gap: 8px; margin-top: 8px; }
  .qr-scan-paste {
    flex: 1; min-width: 0; background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; color: var(--text); padding: 7px 10px; font-size: 0.82rem;
  }
  .qr-scan-paste:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  .qr-scan-use {
    background: color-mix(in srgb, var(--accent) 12%, var(--surface-2));
    border: 1px solid var(--accent); border-radius: 6px; padding: 6px 14px;
    font-size: 0.82rem; color: var(--accent); cursor: pointer;
  }
  .qr-scan-use:disabled { opacity: 0.5; cursor: default; }
</style>
