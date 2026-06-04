<script lang="ts">
  import { api } from '../lib/api'
  import type { WalletEntry, BackendEntry } from '../lib/types'
  import PolicySignerCard, { type PolicySig } from './PolicySignerCard.svelte'
  import BusyButton from './ui/BusyButton.svelte'

  let { label, onCreated, savedBackends = [] }: {
    label: string
    onCreated: (e: WalletEntry) => void
    savedBackends?: BackendEntry[]
  } = $props()

  let backend = $state<string | null>(null)

  const blank = (): PolicySig => ({ fingerprint: '', path: '', xpub: '', accountIndex: 0 })

  type Template = 'vault' | 'timelocked'
  let template = $state<Template>('vault')

  // wsh (SegWit v0) or taproot. Taproot vaults put the single primary key on
  // the key-path and hide recovery in a tapleaf; taproot savings use a NUMS
  // internal key. Taproot vaults therefore allow only one primary key.
  type ScriptKind = 'wsh' | 'taproot'
  let scriptKind = $state<ScriptKind>('wsh')
  let hwAccount = $derived(scriptKind === 'taproot' ? 'taproot' as const : 'multisig_p2wsh' as const)
  let pathPlaceholder = $derived(scriptKind === 'taproot' ? "m/86'/0'/0'" : "m/48'/0'/0'/2'")

  function setScriptKind(k: ScriptKind) {
    scriptKind = k
    // Taproot vault key-path is a single key — collapse extra primaries.
    if (k === 'taproot' && template === 'vault' && primary.length > 1) {
      primary = [primary[0]]
      threshold = 1
    }
  }

  // Vault: primary N-of-M (spend anytime) OR recovery k-of-j (after timelock).
  let threshold = $state(1)
  let primary = $state<PolicySig[]>([blank()])
  let recoveryThreshold = $state(1)
  let recovery = $state<PolicySig[]>([blank()])

  // Timelocked savings: a single key, hard-locked until the timelock.
  let single = $state<PolicySig>(blank())

  let timelockMode = $state<'blocks' | 'height'>('blocks')
  let timelockValue = $state(26280) // ~6 months
  let error = $state('')

  function addPrimary() { primary = [...primary, blank()] }
  function removePrimary(i: number) {
    if (primary.length <= 1) return
    primary = primary.filter((_, idx) => idx !== i)
    if (threshold > primary.length) threshold = primary.length
  }
  function addRecovery() { recovery = [...recovery, blank()] }
  function removeRecovery(i: number) {
    if (recovery.length <= 1) return
    recovery = recovery.filter((_, idx) => idx !== i)
    if (recoveryThreshold > recovery.length) recoveryThreshold = recovery.length
  }

  const sigOk = (s: PolicySig) =>
    /^[0-9a-fA-F]{8}$/.test(s.fingerprint.trim()) && s.path.trim().length > 0 && s.xpub.trim().length > 0

  let timelockHint = $derived.by(() => {
    if (timelockMode !== 'blocks' || timelockValue <= 0) return ''
    const days = (timelockValue * 10) / 1440
    if (days >= 365) return `≈ ${(days / 365).toFixed(1)} years`
    if (days >= 30) return `≈ ${Math.round(days / 30)} months`
    return `≈ ${Math.round(days)} days`
  })

  // A recovery key reused as a primary key makes the descriptor non-standard
  // (BDK rejects repeated pubkeys); warn before the server does.
  let reusedKeyWarning = $derived.by(() => {
    if (template !== 'vault') return ''
    const pset = new Set(primary.map(s => s.xpub.trim()).filter(Boolean))
    return recovery.some(s => s.xpub.trim() && pset.has(s.xpub.trim()))
      ? 'A recovery key matches a primary key — recovery keys must be distinct.'
      : ''
  })

  let timelockOk = $derived(
    timelockValue > 0 && !(timelockMode === 'blocks' && timelockValue > 65535)
  )

  let canCreate = $derived.by(() => {
    if (!label.trim() || !timelockOk) return false
    if (template === 'timelocked') return sigOk(single)
    // vault
    if (threshold < 1 || threshold > primary.length) return false
    if (recoveryThreshold < 1 || recoveryThreshold > recovery.length) return false
    if (!primary.every(sigOk) || !recovery.every(sigOk)) return false
    if (reusedKeyWarning) return false
    return true
  })

  function clean(s: PolicySig) {
    return { fingerprint: s.fingerprint.trim(), path: s.path.trim(), xpub: s.xpub.trim() }
  }

  async function create() {
    if (!canCreate) return
    error = ''
    const tl = {
      timelock_blocks: timelockMode === 'blocks' ? timelockValue : null,
      timelock_height: timelockMode === 'height' ? timelockValue : null,
    }
    const taproot = scriptKind === 'taproot'
    try {
      const entry = template === 'vault'
        ? await api.wallets.createVault({
            label: label.trim(),
            threshold,
            primary: primary.map(clean),
            recovery_threshold: recoveryThreshold,
            recovery: recovery.map(clean),
            taproot,
            backend,
            ...tl,
          })
        : await api.wallets.createTimelocked({
            label: label.trim(),
            signer: clean(single),
            taproot,
            backend,
            ...tl,
          })
      onCreated(entry)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to create wallet'
    }
  }
</script>

<div class="policy">
  <div class="tmpl-tabs">
    <button type="button" class:active={template === 'vault'} onclick={() => template = 'vault'}>
      Vault (recovery)
    </button>
    <button type="button" class:active={template === 'timelocked'} onclick={() => template = 'timelocked'}>
      Timelocked savings
    </button>
  </div>

  <div class="script-toggle">
    <button type="button" class:active={scriptKind === 'wsh'} onclick={() => setScriptKind('wsh')}>SegWit</button>
    <button type="button" class:active={scriptKind === 'taproot'} onclick={() => setScriptKind('taproot')}>Taproot</button>
  </div>

  {#if template === 'vault'}
    {#if scriptKind === 'taproot'}
      <p class="blurb">
        <strong>Taproot vault.</strong> One <strong>primary</strong> key spends anytime via the key-path —
        on-chain it looks like an ordinary payment, and the recovery branch stays hidden until used. A separate
        <strong>recovery</strong> group spends only after the timelock.
        <br /><span class="caveat">Note: the recovery spend is a tapleaf (script-path) — hardware-wallet support is uneven, so it may need software signing.</span>
      </p>
    {:else}
      <p class="blurb">
        <strong>Primary</strong> keys (N-of-M) spend anytime; a separate <strong>recovery</strong> group
        (k-of-j, distinct keys) spends only after the timelock — for inheritance or key-loss recovery.
      </p>
    {/if}

    <div class="sig-head">
      <span class="lbl">{scriptKind === 'taproot' ? 'Primary key (key-path)' : 'Primary signers'}</span>
      {#if scriptKind !== 'taproot'}
        <button type="button" class="add" onclick={addPrimary}>+ Add signer</button>
      {/if}
    </div>
    {#if primary.length > 1}
      <div class="row">
        <label class="lbl" for="v-thresh">Require</label>
        <input id="v-thresh" class="num" type="number" min="1" max={primary.length} bind:value={threshold} />
        <span class="lbl">of {primary.length} to spend</span>
      </div>
    {/if}
    {#each primary as _s, i (i)}
      <PolicySignerCard bind:signer={primary[i]} title={`Primary ${i + 1}`} {hwAccount} {pathPlaceholder} onRemove={primary.length > 1 ? () => removePrimary(i) : undefined} />
    {/each}

    <div class="sig-head">
      <span class="lbl">Recovery keys</span>
      <button type="button" class="add" onclick={addRecovery}>+ Add key</button>
    </div>
    {#if recovery.length > 1}
      <div class="row">
        <label class="lbl" for="r-thresh">Require</label>
        <input id="r-thresh" class="num" type="number" min="1" max={recovery.length} bind:value={recoveryThreshold} />
        <span class="lbl">of {recovery.length} after timelock</span>
      </div>
    {/if}
    {#each recovery as _s, i (i)}
      <PolicySignerCard bind:signer={recovery[i]} title={`Recovery ${i + 1}`} recovery {hwAccount} {pathPlaceholder} onRemove={recovery.length > 1 ? () => removeRecovery(i) : undefined} />
    {/each}

    {#if reusedKeyWarning}<p class="warn">⚠ {reusedKeyWarning}</p>{/if}
  {:else}
    <p class="blurb">
      A single key whose funds are <strong>locked until the timelock</strong> — a savings stash you can't
      spend early. Watch-only; sign once the timelock elapses.
      {#if scriptKind === 'taproot'}<br /><span class="caveat">Taproot: the timelock lives in a tapleaf (no key-path), so spending is script-path — may need software signing.</span>{/if}
    </p>
    <PolicySignerCard bind:signer={single} title="Key" {hwAccount} pathPlaceholder={scriptKind === 'taproot' ? "m/86'/0'/0'" : "m/84'/0'/0'"} />
  {/if}

  <div class="sig-head"><span class="lbl">{template === 'timelocked' ? 'Unlock after' : 'Recovery timelock'}</span></div>
  <div class="timelock-row">
    <div class="mode-toggle">
      <button type="button" class:active={timelockMode === 'blocks'} onclick={() => timelockMode = 'blocks'}>After N blocks</button>
      <button type="button" class:active={timelockMode === 'height'} onclick={() => timelockMode = 'height'}>At block height</button>
    </div>
    <input class="num wide" type="number" min="1" bind:value={timelockValue} />
    {#if timelockMode === 'blocks'}<span class="tl-hint">{timelockHint}{#if timelockValue > 65535} — max 65535 (~1.25y); use block height for longer{/if}</span>{/if}
  </div>

  <div class="backend-row">
    <label for="vault-backend">Backend</label>
    <select id="vault-backend" bind:value={backend}>
      <option value={null}>Default</option>
      {#each savedBackends as b (b.id)}
        <option value={b.id}>{b.label}</option>
      {/each}
    </select>
    <span class="tl-hint">Which server this wallet uses. Add backends in Backend settings.</span>
  </div>

  {#if error}<p class="err">{error}</p>{/if}

  <div class="actions">
    <BusyButton idle={template === 'vault' ? 'Create vault' : 'Create wallet'} busyLabel="Creating…" disabled={!canCreate} onclick={create} />
  </div>
</div>

<style>
  .policy { display: flex; flex-direction: column; gap: 10px; }
  .tmpl-tabs { display: flex; gap: 0; margin-bottom: 2px; }
  .tmpl-tabs button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border);
    color: var(--text-muted); padding: 9px 12px; cursor: pointer; font-size: 0.82rem;
    position: relative;
  }
  .tmpl-tabs button:first-child { border-radius: 6px 0 0 6px; }
  .tmpl-tabs button:last-child { border-radius: 0 6px 6px 0; margin-left: -1px; }
  /* Raise the active tab so its full accent border shows on all four sides
     rather than being clipped by the neighbour's overlapping edge. */
  .tmpl-tabs button.active { background: var(--surface-active); color: var(--text); border-color: var(--accent); z-index: 1; }
  .script-toggle { display: flex; gap: 6px; }
  .script-toggle button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border);
    color: var(--text-muted); padding: 6px 10px; cursor: pointer; font-size: 0.78rem;
    border-radius: 5px;
  }
  .script-toggle button.active { background: var(--surface-active); color: var(--text); border-color: var(--accent); }
  .blurb { font-size: 0.8rem; color: var(--text-muted); line-height: 1.55; margin: 0; }
  .blurb strong { color: var(--text); }
  .caveat { color: #e09c52; font-size: 0.76rem; }
  .lbl { font-size: 0.76rem; color: var(--text-muted); font-weight: 600; }
  .row { display: flex; align-items: center; gap: 10px; margin-top: 4px; }
  .num { width: 64px; background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px; color: var(--text); padding: 7px 9px; font-size: 0.85rem; text-align: center; }
  .num.wide { width: 120px; text-align: left; }
  .num::-webkit-inner-spin-button, .num::-webkit-outer-spin-button { -webkit-appearance: none; margin: 0; }
  .num { -moz-appearance: textfield; appearance: textfield; }
  .sig-head { display: flex; align-items: center; justify-content: space-between; margin-top: 4px; }
  .add { background: none; border: none; color: var(--accent); cursor: pointer; font-size: 0.78rem; padding: 0; }
  .add:hover { text-decoration: underline; }
  .timelock-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .mode-toggle { display: flex; }
  .mode-toggle button { background: var(--surface-2); border: 1px solid var(--border); color: var(--text-muted); padding: 7px 11px; cursor: pointer; font-size: 0.78rem; position: relative; }
  .mode-toggle button:first-child { border-radius: 5px 0 0 5px; }
  .mode-toggle button:last-child { border-radius: 0 5px 5px 0; margin-left: -1px; }
  .mode-toggle button.active { background: var(--surface-active); color: var(--text); border-color: var(--accent); z-index: 1; }
  .tl-hint { font-size: 0.76rem; color: var(--text-muted); }
  .warn { font-size: 0.8rem; color: #e09c52; margin: 0; }
  .err { font-size: 0.8rem; color: var(--error); margin: 0; }
  .backend-row { display: flex; flex-direction: column; gap: 5px; }
  .backend-row label { font-size: 0.82rem; color: var(--text); font-weight: 500; }
  .backend-row select {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-sm, 4px);
    color: var(--text); padding: 7px 10px; font-size: 0.88rem; font-family: inherit; outline: none;
  }
  .backend-row select:focus { border-color: var(--accent); }
  .actions { display: flex; justify-content: flex-end; margin-top: 6px; }
</style>
