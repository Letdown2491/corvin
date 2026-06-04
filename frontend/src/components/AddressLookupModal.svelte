<script lang="ts">
  import type { AddressInfo, WalletEntry } from '../lib/types'
  import { parseDerivation } from '../lib/derivation'
  import Modal from './ui/Modal.svelte'

  let {
    wallet,
    addresses,
    onClose,
  }: {
    wallet: WalletEntry
    addresses: AddressInfo[]
    onClose: () => void
  } = $props()

  let query = $state('')

  // Look up the address in the wallet's derived address set. The
  // wallet's own derivation info (script type, base path, fingerprint)
  // comes from the descriptor. The match info (chain + index) comes
  // from the AddressInfo list — which is already populated for the
  // Addresses tab so this is a zero-roundtrip client-side lookup.
  let derivation = $derived(parseDerivation(wallet))
  let result = $derived.by(() => {
    const q = query.trim()
    if (!q) return null
    const match = addresses.find(a => a.address.toLowerCase() === q.toLowerCase())
    if (!match) return { found: false as const, query: q }

    // Build the full per-address derivation. For single-sig wallets we
    // know the base path (m/84'/0'/0' etc.) plus the chain (0=receive,
    // 1=change) plus the index. For multisig the path is the same
    // suffix appended to each cosigner's branch.
    const chain = match.kind === 'external' ? 0 : 1
    const baseInfo = derivation?.cosigners[0]
    const fullPath = baseInfo ? `${baseInfo.path}/${chain}/${match.index}` : null

    return {
      found: true as const,
      address:  match.address,
      kind:     match.kind,
      index:    match.index,
      used:     match.used,
      tx_count: match.tx_count,
      fullPath,
      scriptType: derivation?.scriptType ?? null,
      fingerprint: baseInfo?.fingerprint ?? null,
      cosigners: derivation && derivation.threshold !== null ? derivation.cosigners : null,
    }
  })

  // (lookup logic above)
</script>

<Modal open onclose={onClose} title="Address derivation lookup" width="560px"
  desc="Paste an address to see whether it belongs to this wallet and where it sits in the derivation tree.">
    <label class="search-label">
      <span class="sr-only">Address to look up</span>
      <input
        class="search-input"
        type="search"
        bind:value={query}
        placeholder="bc1q… or bc1p…"
        spellcheck="false"
        autocapitalize="off"
        autocomplete="off"
      />
    </label>

    {#if !result}
      <p class="hint">Enter an address to look it up.</p>
    {:else if !result.found}
      <div class="not-found">
        <div class="result-icon negative" aria-hidden="true">✕</div>
        <div>
          <div class="result-title">Not derived by this wallet</div>
          <p class="result-desc">
            The address <code>{result.query}</code> doesn't match any address Corvin
            has derived from this wallet's descriptor. It either belongs to a
            different wallet, hasn't been derived yet (past the gap limit), or
            isn't a Bitcoin address at all.
          </p>
        </div>
      </div>
    {:else}
      <div class="found">
        <div class="result-row">
          <span class="result-icon positive" aria-hidden="true">✓</span>
          <div class="result-title">Derived by this wallet</div>
        </div>

        <dl class="result-grid">
          <dt>Chain</dt>
          <dd>{result.kind === 'external' ? 'Receive (external)' : 'Change (internal)'}</dd>

          <dt>Index</dt>
          <dd class="mono">{result.index}</dd>

          {#if result.fullPath}
            <dt>Full path</dt>
            <dd class="mono">{result.fullPath}</dd>
          {/if}

          {#if result.fingerprint}
            <dt>Fingerprint</dt>
            <dd class="mono accent">[{result.fingerprint}]</dd>
          {/if}

          {#if result.scriptType}
            <dt>Script</dt>
            <dd>{result.scriptType.label} ({result.scriptType.bip})</dd>
          {/if}

          <dt>Status</dt>
          <dd>
            {#if result.used}
              Used — {result.tx_count} transaction{result.tx_count === 1 ? '' : 's'}
            {:else}
              Unused — never received funds
            {/if}
          </dd>
        </dl>

        {#if result.cosigners}
          <p class="multisig-note">
            Multisig wallet — each cosigner derives this same address at their
            own corresponding path. Verify each cosigner's
            <code>{result.fullPath?.replace(/^m/, '<cosigner>')}</code>
            against their device to confirm the match.
          </p>
        {/if}
      </div>
    {/if}
</Modal>

<style>
  .search-label { display: block; }
  .search-input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2);
    border: 1px solid var(--border); border-radius: 6px;
    color: var(--text); font-family: monospace; font-size: 0.85rem;
    padding: 8px 10px;
    transition: border-color 0.12s;
  }
  .search-input:focus { outline: none; border-color: var(--accent); }
  .search-input::placeholder { color: var(--text-muted); }

  .hint {
    margin: 14px 0 0; font-size: 0.82rem; color: var(--text-muted);
    text-align: center;
  }

  /* Negative result: low-key warning, not alarming red — most "not found"
     cases are mundane (wrong address, gap-limit, different wallet). */
  .not-found {
    display: flex; gap: 12px; margin-top: 14px;
    padding: 14px; border: 1px solid var(--border); border-radius: 6px;
    background: var(--surface-2);
  }
  .result-icon {
    flex-shrink: 0;
    display: inline-flex; align-items: center; justify-content: center;
    width: 28px; height: 28px; border-radius: 6px;
    font-size: 0.95rem; font-weight: 700;
  }
  .result-icon.negative {
    background: color-mix(in srgb, var(--text-muted) 20%, transparent);
    color: var(--text-muted);
  }
  .result-icon.positive {
    background: color-mix(in srgb, #52a875 18%, transparent);
    color: #52a875;
  }
  .result-title { font-weight: 600; color: var(--text); margin-bottom: 4px; }
  .result-desc { margin: 0; font-size: 0.82rem; color: var(--text-muted); line-height: 1.5; }
  .result-desc code, .multisig-note code {
    background: var(--surface-2); padding: 1px 5px; border-radius: 3px;
    font-size: 0.78rem; color: var(--text);
  }

  .found {
    margin-top: 14px;
    padding: 14px; border: 1px solid color-mix(in srgb, #52a875 35%, var(--border));
    border-radius: 6px; background: var(--surface-2);
  }
  .result-row { display: flex; align-items: center; gap: 12px; margin-bottom: 10px; }

  /* Definition list lays out as label/value pairs. Two-column grid keeps
     the labels aligned and gives the values clear visual ownership. */
  .result-grid {
    display: grid; grid-template-columns: 110px 1fr; gap: 6px 12px;
    margin: 0;
  }
  .result-grid dt {
    font-size: 0.72rem; color: var(--text-muted);
    text-transform: uppercase; letter-spacing: 0.04em;
    align-self: center;
  }
  .result-grid dd {
    margin: 0; font-size: 0.84rem; color: var(--text);
    word-break: break-all;
  }
  /* Narrow screens: single column so the long mono values (paths,
     fingerprints, addresses) get full width and stop wrapping mid-token. */
  @media (max-width: 480px) {
    .result-grid { grid-template-columns: 1fr; gap: 2px 0; }
    .result-grid dt { margin-top: 8px; }
    .result-grid dt:first-of-type { margin-top: 0; }
  }
  .mono { font-family: monospace; font-size: 0.8rem; }
  .accent { color: var(--accent); }

  .multisig-note {
    margin: 10px 0 0;
    font-size: 0.78rem; color: var(--text-muted); line-height: 1.5;
  }

  .sr-only {
    position: absolute; width: 1px; height: 1px; padding: 0; margin: -1px;
    overflow: hidden; clip: rect(0,0,0,0); white-space: nowrap; border: 0;
  }
</style>
