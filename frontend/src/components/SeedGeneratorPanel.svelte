<script lang="ts">
  import { api } from '../lib/api'
  import { invokeDesktop } from '../lib/utils'

  let {
    mnemonic = $bindable(''),
    verified = $bindable(false),
    generated = $bindable(false),
    walletLabel = '',
  }: {
    mnemonic?: string
    verified?: boolean
    generated?: boolean
    /// Used in the Download .txt / Print headers so the file is identifiable
    /// later. Empty string falls back to a generic name.
    walletLabel?: string
  } = $props()

  let wordCount = $state<12 | 24>(12)
  let generating = $state(false)
  let revealed = $state(false)
  let confirmed = $state(false)
  let confirmRegen = $state(false)
  let copied = $state(false)
  let copyTimer: ReturnType<typeof setTimeout> | null = null
  let challengePositions = $state<number[]>([])
  let challengeInputs = $state<string[]>([])
  let errorMsg = $state('')

  let words = $derived(
    mnemonic.trim() ? mnemonic.trim().split(/\s+/).filter(Boolean) : []
  )

  // Verified = wrote-down checkbox + all 3 challenge words match.
  // Catches users who blindly check the box without actually backing up.
  $effect(() => {
    verified = confirmed
      && challengePositions.length === 3
      && challengePositions.every((pos, i) =>
        challengeInputs[i]?.trim().toLowerCase() === words[pos]?.toLowerCase()
      )
  })

  function startChallenge() {
    const idx = Array.from({ length: words.length }, (_, i) => i)
    // Fisher-Yates shuffle, take first 3, sort ascending for less-jarring UX.
    for (let i = idx.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1))
      ;[idx[i], idx[j]] = [idx[j], idx[i]]
    }
    challengePositions = idx.slice(0, 3).sort((a, b) => a - b)
    challengeInputs = ['', '', '']
  }

  async function doGenerate() {
    generating = true; errorMsg = ''; confirmRegen = false
    try {
      const res = await api.wallets.seedGenerate(wordCount)
      mnemonic = res.mnemonic
      generated = true
      revealed = false
      confirmed = false
      challengePositions = []
      challengeInputs = []
    } catch (e) {
      errorMsg = e instanceof Error ? e.message : 'Failed to generate mnemonic'
    } finally {
      generating = false
    }
  }

  function onGenerateClick() {
    // First click: just generate. Subsequent click ("Regenerate") asks for
    // confirmation so a written-down mnemonic isn't silently replaced.
    if (generated && !confirmRegen) { confirmRegen = true; return }
    doGenerate()
  }

  function setWordCount(n: 12 | 24) {
    wordCount = n
    mnemonic = ''
    generated = false
    confirmed = false
    confirmRegen = false
  }

  async function copy() {
    try {
      await navigator.clipboard.writeText(mnemonic)
      copied = true
      if (copyTimer) clearTimeout(copyTimer)
      copyTimer = setTimeout(() => copied = false, 2000)
    } catch {
      errorMsg = 'Clipboard unavailable — write the words down manually.'
    }
  }

  function download() {
    const lines = [
      `Corvin seed phrase — ${walletLabel.trim() || 'wallet'}`,
      `Generated ${new Date().toISOString()}`,
      '',
      ...words.map((w, i) => `${i + 1}. ${w}`),
      '',
      'KEEP THIS FILE OFFLINE AND SECURE.',
      'Anyone with these words can spend the wallet.',
    ]
    const blob = new Blob([lines.join('\n')], { type: 'text/plain' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    const slug = (walletLabel.trim() || 'corvin-wallet').toLowerCase().replace(/[^a-z0-9]+/g, '-')
    a.download = `${slug}-seed.txt`
    a.click()
    URL.revokeObjectURL(url)
  }

  function print() {
    const title = walletLabel.trim() || 'Corvin wallet'
    const esc = (s: string) =>
      s.replace(/[&<>"']/g, (c) =>
        ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] as string,
      )
    const wordsHtml = words
      .map((w, i) => `<li><span class="num">${i + 1}.</span> <span class="word">${esc(w)}</span></li>`)
      .join('')

    // Print from a container in the live document rather than a popup window:
    // window.open is blocked in the desktop webview (and the page's strict CSP
    // would block an iframe too). A print-only stylesheet hides the rest of the
    // app so only the phrase prints; both nodes are removed right after.
    const style = document.createElement('style')
    style.textContent = `
      #seed-print-root { display: none; }
      @media print {
        body > *:not(#seed-print-root) { display: none !important; }
        #seed-print-root {
          display: block !important;
          font-family: ui-sans-serif, system-ui, sans-serif;
          padding: 32px; color: #000;
        }
        #seed-print-root h1 { font-size: 16pt; margin: 0 0 16px; }
        #seed-print-root .gen { font-size: 9pt; color: #555; margin-bottom: 24px; }
        #seed-print-root ol {
          columns: ${wordCount === 12 ? 3 : 4}; column-gap: 24px;
          list-style: none; padding: 0; margin: 0 0 24px;
        }
        #seed-print-root li {
          break-inside: avoid; font-family: monospace; font-size: 13pt; padding: 6px 0;
        }
        #seed-print-root .num { color: #777; display: inline-block; width: 2.2em; }
        #seed-print-root .word { font-weight: 600; }
        #seed-print-root .warn {
          font-size: 9pt; color: #777; border-top: 1px solid #ccc;
          padding-top: 12px; line-height: 1.5;
        }
      }
    `
    const root = document.createElement('div')
    root.id = 'seed-print-root'
    root.innerHTML = `
      <h1>${esc(title)} — seed phrase</h1>
      <p class="gen">Generated ${esc(new Date().toLocaleString())}</p>
      <ol>${wordsHtml}</ol>
      <p class="warn">KEEP THIS PAPER OFFLINE AND SECURE. Anyone who reads these words can spend the wallet.</p>
    `

    const cleanup = () => {
      root.remove()
      style.remove()
      window.removeEventListener('afterprint', cleanup)
    }
    document.body.append(style, root)

    // Desktop: window.print() is a no-op in WebKitGTK (it never emits the print
    // signal), so trigger the native print dialog through the backend. It runs
    // async and renders the DOM when the user confirms, so we can't tear down
    // on completion — keep the (screen-hidden) print nodes in place and remove
    // them on a generous timer instead.
    const desktopPrint = invokeDesktop('print')
    if (desktopPrint) {
      desktopPrint.catch((e) => console.error('print failed', e))
      setTimeout(cleanup, 120000)
      return
    }

    window.addEventListener('afterprint', cleanup)
    try {
      window.print()
    } catch {
      errorMsg = 'Printing is not available here.'
      cleanup()
      return
    }
    // Fallback if afterprint never fires (some webviews don't emit it).
    setTimeout(cleanup, 60000)
  }

  function onConfirmChange() {
    if (confirmed && challengePositions.length === 0) startChallenge()
  }
</script>

<div class="seed-row" style="align-items:flex-end">
  <div class="field">
    <span class="field-label">Word count</span>
    <div class="word-count-btns" role="group" aria-label="Word count">
      <button type="button" class:active={wordCount === 12} onclick={() => setWordCount(12)}>12</button>
      <button type="button" class:active={wordCount === 24} onclick={() => setWordCount(24)}>24</button>
    </div>
  </div>
  <button type="button" class="btn-generate" onclick={onGenerateClick} disabled={generating}>
    {generating ? 'Generating…' : generated ? 'Regenerate' : 'Generate'}
  </button>
</div>

{#if confirmRegen && generated}
  <div class="regen-confirm">
    <span class="regen-warn-icon">⚠</span>
    <div class="regen-confirm-body">
      <div class="regen-confirm-title">Replace the current seed phrase?</div>
      <div class="regen-confirm-msg">If you wrote down the words above, the new ones will be different. The current mnemonic will be lost.</div>
      <div class="regen-confirm-actions">
        <button type="button" class="btn-ghost" onclick={() => confirmRegen = false}>Keep current</button>
        <button type="button" class="btn-danger" onclick={doGenerate}>Yes, regenerate</button>
      </div>
    </div>
  </div>
{/if}

{#if generated && words.length > 0}
  <div class="mnemonic-container">
    <ol class="mnemonic-grid cols-{wordCount === 12 ? 3 : 4}" class:blurred={!revealed}>
      {#each words as word, i (i)}
        <li class="mnemonic-word"><span class="word-num">{i + 1}.</span>{word}</li>
      {/each}
    </ol>
    {#if !revealed}
      <button type="button" class="reveal-overlay" onclick={() => revealed = true} aria-label="Reveal seed phrase">
        <span class="reveal-icon">👁</span>
        <span class="reveal-text">Click to reveal seed phrase</span>
        <span class="reveal-hint">Make sure no-one can see your screen</span>
      </button>
    {/if}
  </div>
  {#if revealed}
    <div class="mnemonic-actions">
      <button type="button" class="action-link" onclick={() => revealed = false}>Hide</button>
      <button type="button" class="action-link" onclick={copy}>{copied ? '✓ Copied' : 'Copy'}</button>
      <button type="button" class="action-link" onclick={download}>Download .txt</button>
      <button type="button" class="action-link" onclick={print}>Print</button>
    </div>
  {/if}
  <p class="seed-warning">Write these words down and store them safely. They will not be shown again.</p>

  <label class="seed-confirm-label">
    <input type="checkbox" bind:checked={confirmed} onchange={onConfirmChange} />
    I have written down my seed phrase in a safe place
  </label>

  {#if confirmed && challengePositions.length === 3}
    <div class="seed-challenge">
      <div class="seed-challenge-header">
        <span class="seed-challenge-title">Verify your backup</span>
        <span class="seed-challenge-hint">Type the requested words to confirm you wrote them down correctly.</span>
      </div>
      <div class="seed-challenge-grid">
        {#each challengePositions as pos, i (i)}
          {@const matched = challengeInputs[i]?.trim().toLowerCase() === words[pos]?.toLowerCase()}
          {@const filled = !!challengeInputs[i]?.trim()}
          <label class="challenge-input-wrap">
            <span class="challenge-pos">Word #{pos + 1}</span>
            <input
              type="text"
              class="challenge-input"
              class:ok={matched}
              class:bad={filled && !matched}
              bind:value={challengeInputs[i]}
              spellcheck="false"
              autocapitalize="off"
              autocomplete="off"
              aria-label="Word number {pos + 1}"
            />
            {#if filled}
              <span class="challenge-feedback">{matched ? '✓' : '✗'}</span>
            {/if}
          </label>
        {/each}
      </div>
      {#if !verified && challengeInputs.some(v => v?.trim())}
        <button type="button" class="action-link" onclick={() => { revealed = true }}>
          Need to see the words again?
        </button>
      {/if}
    </div>
  {/if}
{/if}

{#if errorMsg}
  <p class="error">{errorMsg}</p>
{/if}

<style>
  .seed-row { display: flex; gap: 10px; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .field-label { font-size: 0.8rem; color: var(--text-muted); }

  .word-count-btns { display: flex; gap: 4px; }
  .word-count-btns button {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; padding: 7px 14px; font-size: 0.82rem;
  }
  .word-count-btns button.active {
    background: var(--surface-active); border-color: var(--accent);
    color: var(--accent); font-weight: 600;
  }

  .btn-generate {
    background: var(--accent); color: #000; border: none; border-radius: 5px;
    padding: 8px 16px; cursor: pointer; font-weight: 600; font-size: 0.85rem;
  }
  .btn-generate:hover { filter: brightness(1.08); }
  .btn-generate:disabled { opacity: 0.5; cursor: not-allowed; }

  .regen-confirm {
    display: flex; gap: 10px; align-items: flex-start;
    background: color-mix(in srgb, #e09c52 8%, var(--surface-1));
    border: 1px solid color-mix(in srgb, #e09c52 30%, transparent);
    border-radius: 6px; padding: 10px 14px;
  }
  .regen-warn-icon { color: #e09c52; font-size: 1.2rem; flex-shrink: 0; line-height: 1.4; }
  .regen-confirm-body { display: flex; flex-direction: column; gap: 8px; flex: 1; }
  .regen-confirm-title { font-size: 0.85rem; font-weight: 600; color: var(--text); }
  .regen-confirm-msg { font-size: 0.78rem; color: var(--text-muted); line-height: 1.45; }
  .regen-confirm-actions { display: flex; gap: 8px; justify-content: flex-end; }
  .btn-ghost {
    background: none; border: 1px solid var(--border); border-radius: 4px;
    color: var(--text-muted); cursor: pointer; padding: 5px 12px; font-size: 0.78rem;
  }
  .btn-ghost:hover { color: var(--text); border-color: var(--text-muted); }
  .btn-danger {
    background: #e05252; color: #fff; border: none; border-radius: 4px;
    padding: 5px 12px; cursor: pointer; font-size: 0.78rem; font-weight: 600;
  }
  .btn-danger:hover { filter: brightness(1.08); }

  .mnemonic-container { position: relative; }
  .mnemonic-grid {
    display: grid; gap: 6px 12px; list-style: none; padding: 12px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    margin: 0;
  }
  .mnemonic-grid.cols-3 { grid-template-columns: repeat(3, 1fr); }
  .mnemonic-grid.cols-4 { grid-template-columns: repeat(4, 1fr); }
  .mnemonic-grid.blurred { filter: blur(8px); user-select: none; }
  .mnemonic-word {
    display: flex; gap: 4px; align-items: baseline;
    font-family: monospace; font-size: 0.85rem; color: var(--text);
  }
  .word-num { color: var(--text-muted); font-size: 0.7rem; min-width: 18px; }

  .reveal-overlay {
    position: absolute; inset: 0;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 6px; background: color-mix(in srgb, var(--surface-1) 70%, transparent);
    border: 1px solid var(--border); border-radius: 6px;
    cursor: pointer; color: var(--text); font-size: 0.85rem; font-weight: 500;
    backdrop-filter: blur(2px);
  }
  .reveal-overlay:hover { background: color-mix(in srgb, var(--surface-1) 60%, transparent); }
  .reveal-icon { font-size: 1.4rem; }
  .reveal-text { font-weight: 600; }
  .reveal-hint { font-size: 0.72rem; color: var(--text-muted); }

  .mnemonic-actions {
    display: flex; gap: 12px; justify-content: flex-end;
    margin-top: 2px;
  }
  .action-link {
    background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: 0.75rem; padding: 2px 4px;
    text-decoration: underline; text-underline-offset: 2px;
  }
  .action-link:hover { color: var(--accent); }

  .seed-warning {
    font-size: 0.75rem; color: #e09c52; margin: 4px 0 8px;
    border-left: 2px solid #e09c52; padding-left: 8px;
  }

  .seed-confirm-label {
    display: flex; align-items: center; gap: 7px;
    font-size: 0.8rem; color: var(--text); cursor: pointer;
    padding: 6px 8px; border: 1px solid var(--border); border-radius: 5px;
    background: var(--surface-2);
  }
  .seed-confirm-label input[type="checkbox"] {
    accent-color: var(--accent); cursor: pointer;
    width: auto; padding: 0; background: transparent;
  }

  .seed-challenge {
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 6px; padding: 12px 14px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .seed-challenge-header { display: flex; flex-direction: column; gap: 2px; }
  .seed-challenge-title { font-size: 0.85rem; font-weight: 600; color: var(--text); }
  .seed-challenge-hint { font-size: 0.74rem; color: var(--text-muted); }
  .seed-challenge-grid {
    display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px;
  }
  .challenge-input-wrap { display: flex; flex-direction: column; gap: 3px; position: relative; }
  .challenge-pos { font-size: 0.7rem; color: var(--text-muted); }
  .challenge-input {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 6px 24px 6px 8px; font-size: 0.82rem;
    font-family: monospace; width: 100%; box-sizing: border-box;
    outline: none;
  }
  .challenge-input:focus { border-color: var(--accent); }
  .challenge-input.ok { border-color: #52a875; }
  .challenge-input.bad { border-color: #e05252; }
  .challenge-feedback {
    position: absolute; right: 8px; bottom: 6px;
    font-size: 0.9rem; pointer-events: none;
  }

  .error { color: #e05252; font-size: 0.82rem; margin: 0; }

  @media (max-width: 520px) {
    .seed-challenge-grid { grid-template-columns: 1fr; }
  }
</style>
