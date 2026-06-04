<script lang="ts">
  // Silent Payments wallet creation (BIP-352). Self-contained like VaultCreatePanel:
  // owns its sub-state, backend pin, validation, and create call; emits the new
  // wallet via onCreated. The parent renders only the shared Name field.
  import { api } from '../lib/api'
  import { nodeStatus } from '../stores/settings'
  import { checkMnemonic, type MnemonicCheck } from '../lib/bip39'
  import type { WalletEntry, BackendEntry } from '../lib/types'
  import SeedGeneratorPanel from './SeedGeneratorPanel.svelte'
  import BusyButton from './ui/BusyButton.svelte'
  import HelpLink from './HelpLink.svelte'

  let { label, onCreated, savedBackends = [] }: {
    label: string
    onCreated: (e: WalletEntry) => void
    savedBackends?: BackendEntry[]
  } = $props()

  let backend = $state<string | null>(null)
  let error = $state('')

  let source = $state<'from_seed' | 'watch_only'>('from_seed')
  // Generate a new seed by default so picking the SP card lands in the safe path.
  let seedMode = $state<'generate' | 'import'>('generate')
  let mnemonic = $state('')
  let passphrase = $state('')
  let accountIndex = $state(0)
  let scanSecretHex = $state('')
  let spendPubkeyHex = $state('')
  // Watch-only birthday: earliest block to scan. Blank = full scan. From-seed
  // wallets use the current chain tip instead.
  let birthday = $state('')
  let seedGenerated = $state(false)
  let seedVerified = $state(false)
  let mnemonicCheck = $state<MnemonicCheck | null>(null)
  let mnemonicCheckSeq = 0
  let showPassphraseHelp = $state(false)

  $effect(() => {
    const m = mnemonic
    if (source !== 'from_seed' || !m.trim()) {
      mnemonicCheck = null
      return
    }
    const seq = ++mnemonicCheckSeq
    checkMnemonic(m).then((c) => {
      if (seq === mnemonicCheckSeq) mnemonicCheck = c
    })
  })

  let canCreate = $derived.by(() => {
    if (!label.trim()) return false
    if (source === 'from_seed') {
      if (seedMode === 'generate') return seedGenerated && seedVerified
      return mnemonicCheck?.kind === 'ok'
    }
    const isHex = (s: string, len: number) => s.length === len && /^[0-9a-fA-F]+$/.test(s)
    return isHex(scanSecretHex.trim(), 64) && isHex(spendPubkeyHex.trim(), 66)
  })

  async function create() {
    if (!canCreate) return
    error = ''
    try {
      let entry
      if (source === 'from_seed') {
        // A freshly-generated/imported seed wallet hasn't received anything before
        // now — scan from the current tip. Falls back to full scan if unknown.
        entry = await api.silentPayments.createWallet({
          label: label.trim(),
          source: 'from_seed',
          mnemonic: mnemonic.trim(),
          passphrase,
          account_index: accountIndex,
          birthday_height: $nodeStatus?.tip_height ?? null,
          backend,
        })
      } else {
        const bd = parseInt(birthday.trim(), 10)
        entry = await api.silentPayments.createWallet({
          label: label.trim(),
          source: 'watch_only',
          scan_secret_hex: scanSecretHex.trim().toLowerCase(),
          spend_pubkey_hex: spendPubkeyHex.trim().toLowerCase(),
          birthday_height: Number.isFinite(bd) && bd >= 0 ? bd : null,
          backend,
        })
      }
      // Zeroize sensitive UI state once it's left this page.
      mnemonic = ''; passphrase = ''; scanSecretHex = ''
      onCreated(entry)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Unknown error'
    }
  }
</script>

<div class="sp-panel">
  <div class="field">
    <label for="sp-backend">Backend</label>
    <select id="sp-backend" bind:value={backend}>
      <option value={null}>Public · frigate.2140.dev</option>
      {#each savedBackends as b (b.id)}
        <option value={b.id}>{b.label}</option>
      {/each}
    </select>
    <p class="field-hint">
      Which Frigate server scans for this wallet. “Public” uses frigate.2140.dev; add a Frigate backend in Backend settings to scan on your own server and keep this wallet's receipts private to it.
    </p>
  </div>

  <div class="method-tabs">
    <button type="button" class:active={source === 'from_seed'} onclick={() => source = 'from_seed'}>
      From seed
    </button>
    <button type="button" class:active={source === 'watch_only'} onclick={() => source = 'watch_only'}>
      Watch-only
    </button>
  </div>

  <div class="method-body">
    {#if source === 'from_seed'}
      <p class="sp-help">
        Scan + spend keys are derived at <code>m/352'/coin'/account'/...</code>. Only the scan secret is stored — the spend secret is re-derived from the mnemonic at send time, never persisted. Same seed as another HD wallet is fine; BIP-352 lives at its own purpose number so the keys are independent.
      </p>

      <div class="seed-row">
        <div class="field" style="width:72px;flex-shrink:0">
          <label for="sp-acct">Account</label>
          <input id="sp-acct" type="number" min="0" max="99" bind:value={accountIndex} />
        </div>
      </div>

      <div class="seed-mode-tabs">
        <button type="button" class:active={seedMode === 'generate'} onclick={() => { seedMode = 'generate'; mnemonic = ''; seedGenerated = false; seedVerified = false }}>Generate new</button>
        <button type="button" class:active={seedMode === 'import'} onclick={() => { seedMode = 'import'; mnemonic = ''; seedGenerated = false; seedVerified = false }}>Import existing</button>
      </div>

      {#if seedMode === 'generate'}
        <SeedGeneratorPanel
          bind:mnemonic={mnemonic}
          bind:verified={seedVerified}
          bind:generated={seedGenerated}
          walletLabel={label}
        />
      {:else}
        <label for="sp-mnemonic" class="field-label">Seed phrase</label>
        <textarea
          id="sp-mnemonic"
          bind:value={mnemonic}
          rows="3"
          placeholder="twelve or twenty-four words"
          spellcheck="false"
          autocapitalize="off"
        ></textarea>
      {/if}

      {#if (seedMode === 'generate' && seedGenerated) || (seedMode === 'import' && mnemonicCheck?.kind === 'ok')}
        <div class="field">
          <label for="sp-pass">
            BIP39 passphrase <span class="optional">(optional)</span>
            <button type="button" class="info-btn" onclick={() => showPassphraseHelp = !showPassphraseHelp} aria-label="What is this?">?</button>
          </label>
          {#if showPassphraseHelp}
            <p class="info-blurb">A passphrase (sometimes called a "25th word") is an extra secret combined with your seed to derive a different wallet. It's optional and not stored anywhere — if you set one, you must remember it to access your funds.</p>
          {/if}
          <input
            id="sp-pass"
            type="password"
            bind:value={passphrase}
            placeholder="Leave blank if unsure"
            autocomplete="new-password"
          />
        </div>
      {/if}
    {:else}
      <p class="sp-help">
        Watch-only: paste someone else's SP keys to monitor incoming payments to their <code>sp1q…</code> address. You can't spend; the spend secret stays with them.
      </p>
      <label for="sp-scan" class="field-label">Scan secret (64 hex)</label>
      <textarea
        id="sp-scan"
        bind:value={scanSecretHex}
        rows="2"
        placeholder="64-character lowercase hex"
        spellcheck="false"
        autocapitalize="off"
      ></textarea>
      <label for="sp-spend" class="field-label">Spend pubkey (66 hex)</label>
      <textarea
        id="sp-spend"
        bind:value={spendPubkeyHex}
        rows="2"
        placeholder="66-character lowercase hex (compressed pubkey)"
        spellcheck="false"
        autocapitalize="off"
      ></textarea>
      <label for="sp-birthday" class="field-label">Birthday block height <span class="optional">(optional)</span></label>
      <input
        id="sp-birthday"
        type="number"
        min="0"
        bind:value={birthday}
        placeholder="Earliest block to scan — leave blank to scan from the start"
      />
      <span class="field-hint">Scanning is expensive. If you know roughly when this wallet first received funds, set a height to skip older blocks.</span>
    {/if}
    <p class="sp-trust">
      <strong>Trust note:</strong> SP scanning runs on the configured Electrum server (Frigate-compatible). The server sees the scan key during the session — they can see what you receive, but not spend. For maximum privacy, self-host Frigate. <HelpLink anchor="sp-concept" />
    </p>
  </div>

  {#if error}<p class="error">{error}</p>{/if}

  <div class="actions">
    <BusyButton idle="Add wallet" busyLabel="Adding…" disabled={!canCreate} onclick={create} />
  </div>
</div>

<style>
  .sp-panel { display: flex; flex-direction: column; gap: 16px; }

  .field { display: flex; flex-direction: column; gap: 5px; }
  .field-label { font-size: 0.8rem; color: var(--text-muted); }
  label[for] { font-size: 0.8rem; color: var(--text-muted); }
  input, textarea, select {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%;
  }
  input:focus, textarea:focus, select:focus { outline: 1px solid var(--accent); }
  textarea { font-family: monospace; resize: none; }
  select { font-family: inherit; }
  input[type="number"] { appearance: textfield; -moz-appearance: textfield; }

  .method-tabs { display: flex; gap: 4px; }
  .method-tabs button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    padding: 7px 10px; cursor: pointer; font-size: 0.8rem; color: var(--text-muted); font-weight: 500;
  }
  .method-tabs button:hover { color: var(--text); }
  .method-tabs button.active {
    background: var(--surface-active); border-color: var(--accent);
    color: var(--accent); font-weight: 600;
  }

  .method-body { display: flex; flex-direction: column; gap: 8px; }
  .field-hint { font-size: 0.75rem; color: var(--text-muted); line-height: 1.4; }
  .optional { font-size: 0.72rem; color: var(--text-muted); font-weight: normal; margin-left: 3px; }

  .seed-mode-tabs { display: flex; gap: 3px; }
  .seed-mode-tabs button {
    flex: 1; background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    padding: 5px 10px; cursor: pointer; font-size: 0.78rem; color: var(--text-muted); font-weight: 500;
  }
  .seed-mode-tabs button.active { background: var(--surface-active); border-color: var(--accent); color: var(--accent); font-weight: 600; }

  .seed-row { display: flex; gap: 10px; align-items: flex-start; }

  .info-btn {
    background: var(--surface-2); border: 1px solid var(--border);
    color: var(--text-muted); cursor: pointer; font-size: 0.7rem;
    width: 16px; height: 16px; border-radius: 50%; padding: 0;
    display: inline-flex; align-items: center; justify-content: center;
    margin-left: 4px; line-height: 1;
  }
  .info-btn:hover { border-color: var(--accent); color: var(--accent); }
  .info-blurb {
    margin: 4px 0 4px;
    font-size: 0.75rem; color: var(--text-muted); line-height: 1.5;
    background: var(--surface-2); border-left: 2px solid var(--accent);
    padding: 6px 10px; border-radius: 0 4px 4px 0;
  }

  .sp-help {
    margin: 0 0 4px 0; font-size: 0.82rem; color: var(--text-muted); line-height: 1.5;
  }
  .sp-help code {
    font-family: monospace; font-size: 0.78rem;
    background: var(--surface-2); padding: 1px 4px; border-radius: 3px;
  }
  .sp-trust {
    margin: 6px 0 0; font-size: 0.78rem; color: var(--text-muted);
    line-height: 1.55;
    padding: 10px 12px;
    background: color-mix(in srgb, var(--accent) 6%, var(--surface-2));
    border: 1px solid color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 6px;
  }
  .sp-trust strong { color: var(--text); }

  .error { color: var(--error); font-size: 0.85rem; margin: 0; }

  .actions { display: flex; justify-content: flex-end; margin-top: 6px; }
</style>
