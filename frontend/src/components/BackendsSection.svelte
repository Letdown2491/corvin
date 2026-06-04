<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import type { BackendEntry } from '../lib/types'
  import { addToast } from '../stores/toasts'

  // `defaultBackendId` = the backend currently selected as the global default
  // (from ServerPage's in-memory settings), so we can flag it in the list.
  let { defaultBackendId = null }: { defaultBackendId?: string | null } = $props()

  let backends = $state<BackendEntry[]>([])
  let loading = $state(true)
  let adding = $state(false)
  let error = $state('')

  // Add/edit form. `editingId` null = adding a new backend; otherwise the id
  // of the saved backend being edited.
  let editingId = $state<string | null>(null)
  let label = $state('')
  // 'frigate' = an Electrum server that also speaks BIP-352 (Silent Payments).
  let uiKind = $state<'electrum' | 'rpc' | 'frigate'>('electrum')
  let electrumHost = $state('')
  let electrumPort = $state(50002)
  let electrumSsl = $state(true)
  let rpcUrl = $state('http://127.0.0.1:8332')
  let rpcUser = $state('')
  let rpcPass = $state('')
  let showAdvanced = $state(false)
  let dangerAcceptInvalidCerts = $state(false)
  let validateTls = $state(true)
  let caCertPath = $state('')
  let socks5Proxy = $state<string | null>(null)
  let testing = $state(false)
  let testResult = $state<{ ok: boolean; msg: string } | null>(null)

  async function load() {
    loading = true
    try {
      backends = await api.backends.list()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load backends'
    } finally {
      loading = false
    }
  }
  onMount(load)

  function resetForm() {
    editingId = null
    label = ''; uiKind = 'electrum'
    electrumHost = ''; electrumPort = 50002; electrumSsl = true
    rpcUrl = 'http://127.0.0.1:8332'; rpcUser = ''; rpcPass = ''
    showAdvanced = false; dangerAcceptInvalidCerts = false; validateTls = true
    caCertPath = ''; socks5Proxy = null
    error = ''; testResult = null
  }

  function startEdit(b: BackendEntry) {
    editingId = b.id
    label = b.label; uiKind = b.frigate ? 'frigate' : b.type
    electrumHost = b.electrum_host; electrumPort = b.electrum_port; electrumSsl = b.electrum_ssl
    rpcUrl = b.rpc_url; rpcUser = b.rpc_user; rpcPass = '' // masked on read; blank keeps the stored one
    dangerAcceptInvalidCerts = b.danger_accept_invalid_certs
    validateTls = b.validate_tls
    caCertPath = b.ca_cert_path ?? ''
    socks5Proxy = b.socks5_proxy ?? null
    showAdvanced = !!(b.danger_accept_invalid_certs || b.ca_cert_path || b.socks5_proxy || !b.validate_tls)
    error = ''; testResult = null
  }

  // Frigate backends connect like Electrum (kind 'electrum') with the SP flag set.
  let isElectrumLike = $derived(uiKind === 'electrum' || uiKind === 'frigate')

  function draft(): BackendEntry {
    return {
      id: editingId ?? crypto.randomUUID(),
      label: label.trim(),
      type: uiKind === 'rpc' ? 'rpc' : 'electrum',
      frigate: uiKind === 'frigate',
      electrum_host: electrumHost.trim(),
      electrum_port: electrumPort,
      electrum_ssl: electrumSsl,
      validate_tls: validateTls,
      ca_cert_path: caCertPath.trim() || null,
      danger_accept_invalid_certs: isElectrumLike && electrumSsl && dangerAcceptInvalidCerts,
      socks5_proxy: socks5Proxy?.trim() || null,
      rpc_url: rpcUrl.trim(),
      rpc_user: rpcUser.trim(),
      rpc_pass: rpcPass,
    }
  }

  function validateDraft(): boolean {
    if (!label.trim()) { error = 'Give the backend a name.'; return false }
    if (isElectrumLike && !electrumHost.trim()) { error = 'Enter the server host.'; return false }
    return true
  }

  async function save() {
    if (!validateDraft()) return
    adding = true; error = ''
    try {
      if (editingId) {
        const updated = await api.backends.update(editingId, draft())
        backends = backends.map(b => b.id === editingId ? updated : b)
        addToast('Backend updated')
      } else {
        const created = await api.backends.create(draft())
        backends = [...backends, created]
        addToast('Backend saved')
      }
      resetForm()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to save backend'
    } finally {
      adding = false
    }
  }

  async function test() {
    if (!validateDraft()) return
    testing = true; error = ''; testResult = null
    try {
      const s = await api.backends.test(draft())
      testResult = s.connected
        ? { ok: true, msg: `Connected · ${s.network}${s.tip_height ? ' · block ' + s.tip_height.toLocaleString() : ''}` }
        : { ok: false, msg: s.error ? `Failed: ${s.error}` : 'Could not connect' }
    } catch (e) {
      testResult = { ok: false, msg: e instanceof Error ? e.message : 'Test failed' }
    } finally {
      testing = false
    }
  }

  async function remove(b: BackendEntry) {
    const isDefault = b.id === defaultBackendId
    const note = isDefault
      ? ' It is your current default backend, so the default reverts to a public server, and'
      : ''
    if (!confirm(`Remove "${b.label}"?${note} any wallet using it falls back to the default backend.`)) return
    try {
      await api.backends.delete(b.id)
      backends = backends.filter(x => x.id !== b.id)
      if (editingId === b.id) resetForm()
      addToast(`Removed "${b.label}"`)
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Failed to remove backend')
    }
  }

  function summary(b: BackendEntry): string {
    const tags: string[] = []
    if (b.type === 'electrum' && b.electrum_ssl && b.danger_accept_invalid_certs) tags.push('self-signed')
    if (b.socks5_proxy) tags.push('Tor')
    const suffix = tags.length ? ` · ${tags.join(' · ')}` : ''
    if (b.type === 'rpc') return `RPC · ${b.rpc_url}${suffix}`
    const proto = `${b.electrum_ssl ? 'ssl' : 'tcp'}://${b.electrum_host}:${b.electrum_port}`
    return `${b.frigate ? 'Frigate' : 'Electrum'} · ${proto}${suffix}`
  }
</script>

<div class="backends">
  <p class="hint">Additional backends a wallet can use instead of the default. Pin one when adding a wallet to keep that wallet's activity off your other backends. Removing one sends any wallet using it back to the default.</p>

  {#if loading}
    <p class="hint">Loading…</p>
  {:else}
    {#if backends.length}
      <ul class="list">
        {#each backends as b (b.id)}
          <li class="row" class:editing={editingId === b.id}>
            <div class="row-main">
              <span class="row-label-row">
                <span class="row-label">{b.label}</span>
                {#if b.id === defaultBackendId}<span class="default-pill">Default</span>{/if}
              </span>
              <span class="row-sub">{summary(b)}</span>
            </div>
            <div class="row-actions">
              <button type="button" class="icon-btn" onclick={() => startEdit(b)} title="Edit backend" aria-label="Edit backend">✎</button>
              <button type="button" class="icon-btn del" onclick={() => remove(b)} title="Remove backend" aria-label="Remove backend">✕</button>
            </div>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="hint">No saved backends yet.</p>
    {/if}

    <!-- Enter saves the backend rather than submitting the surrounding settings form. -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="add" role="group"
      onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); save() } }}
      oninput={() => testResult = null}
      onchange={() => testResult = null}
    >
      <div class="form-head">
        <span class="form-title">{editingId ? 'Edit backend' : 'Add a backend'}</span>
        {#if editingId}
          <button type="button" class="btn-secondary" onclick={resetForm}>Cancel</button>
        {/if}
      </div>
      <div class="field">
        <label for="b-label">Name</label>
        <input id="b-label" bind:value={label} placeholder="My node" autocomplete="off" />
      </div>
      <div class="field">
        <label for="b-type">Type</label>
        <select id="b-type" bind:value={uiKind}>
          <option value="electrum">Electrum server</option>
          <option value="rpc">Bitcoin node (RPC)</option>
          <option value="frigate">Frigate (Silent Payments)</option>
        </select>
      </div>

      {#if isElectrumLike}
        <div class="field-row">
          <div class="field grow">
            <label for="b-host">Host</label>
            <input id="b-host" bind:value={electrumHost} placeholder={uiKind === 'frigate' ? 'frigate.2140.dev' : '127.0.0.1'} autocomplete="off" />
          </div>
          <div class="field port">
            <label for="b-port">Port</label>
            <input id="b-port" type="number" bind:value={electrumPort} />
          </div>
          <label class="ssl"><input type="checkbox" bind:checked={electrumSsl} /> SSL</label>
        </div>
      {:else}
        <div class="field">
          <label for="b-url">RPC URL</label>
          <input id="b-url" bind:value={rpcUrl} autocomplete="off" />
        </div>
        <div class="field-row">
          <div class="field grow">
            <label for="b-user">RPC user</label>
            <input id="b-user" bind:value={rpcUser} autocomplete="off" />
          </div>
          <div class="field grow">
            <label for="b-pass">RPC password</label>
            <input id="b-pass" type="password" bind:value={rpcPass} placeholder={editingId ? 'unchanged' : ''} autocomplete="off" />
          </div>
        </div>
      {/if}

      <button type="button" class="adv-toggle" onclick={() => showAdvanced = !showAdvanced}>
        <span class="adv-caret" class:open={showAdvanced}>▸</span> Advanced (TLS, Tor proxy)
      </button>

      {#if showAdvanced}
        <div class="advanced-block">
          {#if isElectrumLike && electrumSsl}
            <label class="check">
              <input type="checkbox" bind:checked={dangerAcceptInvalidCerts} />
              <span>Accept invalid / self-signed certificates <span class="warn">— for your own server</span></span>
            </label>
            {#if !dangerAcceptInvalidCerts}
              <label class="check">
                <input type="checkbox" bind:checked={validateTls} />
                <span>Strict hostname check</span>
              </label>
              <div class="field">
                <label for="b-ca">CA certificate path <span class="optional">(optional)</span></label>
                <input id="b-ca" bind:value={caCertPath} placeholder="/path/to/electrum-server-ca.pem" autocomplete="off" />
              </div>
            {/if}
          {/if}

          <label class="check">
            <input
              type="checkbox"
              checked={socks5Proxy !== null}
              onchange={(e) => socks5Proxy = (e.target as HTMLInputElement).checked ? '127.0.0.1:9050' : null}
            />
            <span>Route through SOCKS5 proxy (Tor)</span>
          </label>
          {#if socks5Proxy !== null}
            <div class="field">
              <label for="b-proxy">Proxy address</label>
              <input id="b-proxy" bind:value={socks5Proxy} placeholder="127.0.0.1:9050" autocomplete="off" />
            </div>
          {/if}
        </div>
      {/if}

      {#if error}<p class="err">{error}</p>{/if}
      {#if testResult}<p class="test-result" class:ok={testResult.ok}>{testResult.msg}</p>{/if}
      <div class="form-actions">
        <button type="button" class="add-btn" onclick={save} disabled={adding || testing}>
          {adding ? 'Saving…' : (editingId ? 'Save changes' : '+ Add backend')}
        </button>
        <button type="button" class="test-btn" onclick={test} disabled={adding || testing}>
          {testing ? 'Testing…' : 'Test connection'}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .backends { display: flex; flex-direction: column; gap: 12px; }
  .hint { font-size: var(--text-sm); color: var(--text-muted); margin: 0; line-height: 1.5; }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .row {
    display: flex; align-items: center; justify-content: space-between; gap: 10px;
    padding: 8px 10px; background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius);
  }
  .row-main { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .row-label-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .row-label { font-size: var(--text-sm); font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .default-pill {
    flex-shrink: 0; font-size: var(--text-xs); font-weight: 600; line-height: 1;
    color: var(--accent); background: color-mix(in srgb, var(--accent) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    padding: 2px 6px; border-radius: 999px;
  }
  .row-sub { font-size: var(--text-xs); color: var(--text-muted); font-family: var(--font-mono); }
  .row.editing { border-color: var(--accent); }
  /* Actions clustered on the right, visually separated from the backend info. */
  .row-actions {
    display: flex; align-items: center; gap: 2px; flex-shrink: 0;
    padding-left: 8px; margin-left: 4px; border-left: 1px solid var(--border);
  }
  .icon-btn {
    background: none; border: none; color: var(--text-muted); cursor: pointer;
    font-size: var(--text-base); line-height: 1; padding: 4px 6px; border-radius: var(--radius-sm);
  }
  .icon-btn:hover { color: var(--accent); background: var(--surface-hover); }
  .icon-btn.del:hover { color: var(--error); }

  .add {
    display: flex; flex-direction: column; gap: 10px;
    padding: 12px; background: var(--surface-2);
    border: 1px solid var(--border); border-radius: var(--radius);
  }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .field label { font-size: var(--text-xs); color: var(--text-muted); }
  .field input, .field select {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: var(--radius-sm);
    color: var(--text); padding: 6px 8px; font-size: var(--text-sm); outline: none;
  }
  .field input:focus, .field select:focus { border-color: var(--accent); }
  .field-row { display: flex; gap: 8px; align-items: flex-end; }
  .field.grow { flex: 1; min-width: 0; }
  .field.port input { width: 84px; }
  .ssl { display: flex; align-items: center; gap: 5px; font-size: var(--text-sm); color: var(--text); padding-bottom: 7px; }
  .adv-toggle {
    align-self: flex-start; background: none; border: none; cursor: pointer;
    color: var(--text-muted); font-size: var(--text-sm); padding: 0; display: inline-flex; align-items: center; gap: 6px;
  }
  .adv-toggle:hover { color: var(--text); }
  .adv-caret { transition: transform 0.15s; display: inline-block; }
  .adv-caret.open { transform: rotate(90deg); }
  .advanced-block { display: flex; flex-direction: column; gap: 10px; padding-left: 4px; }
  .check { display: flex; align-items: flex-start; gap: 7px; font-size: var(--text-sm); color: var(--text); cursor: pointer; }
  .check input { margin-top: 2px; flex-shrink: 0; }
  .warn { color: var(--text-muted); font-weight: 400; }
  .optional { color: var(--text-muted); font-weight: 400; }
  .err { font-size: var(--text-sm); color: var(--error); margin: 0; }
  .form-head { display: flex; align-items: center; justify-content: space-between; }
  .form-title { font-size: var(--text-sm); font-weight: 600; color: var(--text); }
  .test-result { font-size: var(--text-sm); color: var(--error); margin: 0; }
  .test-result.ok { color: var(--success, #52a875); }
  .form-actions { display: flex; gap: 8px; align-items: center; }
  .add-btn, .test-btn {
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: var(--radius-sm); color: var(--text); cursor: pointer;
    font-size: var(--text-sm); padding: 7px 14px;
  }
  .add-btn:hover:not(:disabled), .test-btn:hover:not(:disabled) { border-color: var(--accent); color: var(--accent); }
  .add-btn:disabled, .test-btn:disabled { opacity: 0.5; cursor: default; }
</style>
