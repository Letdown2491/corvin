<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import type { WalletEntry, BackendEntry } from '../lib/types'
  import { PUBLIC_SERVERS } from '../lib/public-servers'
  import { addToast } from '../stores/toasts'
  import BusyButton from './ui/BusyButton.svelte'
  import Modal from './ui/Modal.svelte'

  let { wallet, onClose, onChanged }: {
    wallet: WalletEntry
    onClose: () => void
    /// Called with the updated entry after a successful change.
    onChanged: (w: WalletEntry) => void
  } = $props()

  let savedBackends = $state<BackendEntry[]>([])
  // 'default' = follow the global default; 'pub:<host>' = a public Electrum
  // server (materialized into a saved backend on save); 'saved:<id>' = a saved one.
  let selectedKey = $state('default')
  let saving = $state(false)
  let error = $state('')

  // SP wallets scan via a Frigate (BIP-352) server; other kinds use a regular one.
  let isSp = $derived(wallet.kind === 'silent_payments')
  let savedOptions = $derived(savedBackends.filter(b => (isSp ? b.frigate : !b.frigate)))
  let initialKey = $derived(wallet.backend ? `saved:${wallet.backend}` : 'default')
  let changed = $derived(selectedKey !== initialKey)

  onMount(async () => {
    selectedKey = wallet.backend ? `saved:${wallet.backend}` : 'default'
    try { savedBackends = await api.backends.list() } catch { /* registry unavailable */ }
  })

  // Resolve the dropdown choice to a backend id to pin (null = follow default).
  // A public-server pick is materialized into the saved-backends registry
  // (deduped by connection) so the wallet can reference it like any other.
  async function resolveBackendId(): Promise<string | null> {
    if (selectedKey === 'default') return null
    if (selectedKey.startsWith('saved:')) return selectedKey.slice('saved:'.length)
    const host = selectedKey.slice('pub:'.length)
    const ps = PUBLIC_SERVERS.find(s => s.host === host)
    if (!ps) return null
    const existing = savedBackends.find(b =>
      b.type === 'electrum' && !b.frigate &&
      b.electrum_host === ps.host && b.electrum_port === ps.port && b.electrum_ssl === ps.ssl,
    )
    if (existing) return existing.id
    const created = await api.backends.create({
      id: crypto.randomUUID(),
      label: ps.host,
      type: 'electrum',
      frigate: false,
      electrum_host: ps.host,
      electrum_port: ps.port,
      electrum_ssl: ps.ssl,
      validate_tls: true,
      ca_cert_path: null,
      danger_accept_invalid_certs: false,
      socks5_proxy: null,
      rpc_url: '',
      rpc_user: '',
      rpc_pass: '',
    })
    return created.id
  }

  async function save() {
    error = ''
    try {
      const id = await resolveBackendId()
      const updated = await api.wallets.setBackend(wallet.id, id)
      onChanged(updated)
      addToast('Backend updated')
      onClose()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to update backend'
    }
  }
</script>

<Modal open onclose={onClose} title="Change backend" width="460px"
  desc={`Which server ${wallet.label} syncs and broadcasts through.`}>
    <div class="form-section">
      <label class="field-label" for="cb-select">{isSp ? 'Scanner' : 'Server'}</label>
      <select id="cb-select" class="text-input" bind:value={selectedKey}>
        {#if isSp}
          <option value="default">Public · frigate.2140.dev</option>
        {:else}
          <option value="default">Default (your default backend)</option>
          <optgroup label="Public Electrum servers">
            {#each PUBLIC_SERVERS as s (s.host)}
              <option value={`pub:${s.host}`}>{s.host}</option>
            {/each}
          </optgroup>
        {/if}
        {#if savedOptions.length}
          <optgroup label={isSp ? 'Your Frigate backends' : 'Your saved backends'}>
            {#each savedOptions as b (b.id)}
              <option value={`saved:${b.id}`}>{b.label}</option>
            {/each}
          </optgroup>
        {/if}
      </select>
      <p class="hint">
        {#if isSp}
          “Public” uses frigate.2140.dev. Add a Frigate backend in Backend settings to scan on your own server.
        {:else}
          “Default” follows your global default backend. Pick a specific server to keep this wallet's activity off your others — add private servers or your node in Backend settings.
        {/if}
      </p>
    </div>

    <BusyButton bind:busy={saving} idle="Save" busyLabel="Saving…" disabled={!changed} onclick={save} />
    {#if error}<div class="error-box">{error}</div>{/if}
</Modal>

<style>
  .form-section { margin-bottom: 12px; }
  .field-label {
    display: block; font-size: 0.74rem; font-weight: 600; color: var(--text-muted);
    margin-bottom: 5px; text-transform: uppercase; letter-spacing: 0.06em;
  }
  .text-input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 4px; color: var(--text); padding: 7px 10px;
    font-size: 0.82rem; outline: none;
  }
  .text-input:focus { border-color: var(--accent); }
  .hint { font-size: 0.78rem; color: var(--text-muted); line-height: 1.5; margin: 6px 0 0; }

  .error-box {
    margin-top: 10px; padding: 8px 10px;
    background: color-mix(in srgb, #e05252 8%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e05252 40%, var(--border));
    border-radius: 4px; font-size: 0.78rem; color: var(--text);
  }
</style>
