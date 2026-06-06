<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { api } from '../lib/api'
  import type { BackendEntry, Settings } from '../lib/types'
  import { mempoolUrl } from '../stores/settings'
  import { PUBLIC_SERVERS, type PublicServer } from '../lib/public-servers'
  import HelpLink from './HelpLink.svelte'
  import BackendsSection from './BackendsSection.svelte'
  import BusyButton from './ui/BusyButton.svelte'
  import { get } from 'svelte/store'
  import { wallets } from '../stores/wallets'
  import { nodeStatus, backendStatuses } from '../stores/settings'
  import { kindLabel } from '../lib/utils'

  let settings = $state<Settings | null>(null)
  let error = $state('')
  // Which section is currently flashing its "Autosaved ✓" header indicator.
  let flashed = $state<string | null>(null)
  let flashTimer: ReturnType<typeof setTimeout> | null = null
  function flash(section: string) {
    flashed = section
    if (flashTimer) clearTimeout(flashTimer)
    flashTimer = setTimeout(() => { flashed = null }, 1500)
  }
  let testing = $state(false)
  let testResult = $state<{ ok: boolean; msg: string } | null>(null)
  let mempoolTesting = $state(false)
  let mempoolTestResult = $state<{ ok: boolean; msg: string } | null>(null)
  // Saved backends from the registry, offered as default choices alongside the
  // public servers. The full connection editor for these lives in BackendsSection.
  let savedBackends = $state<BackendEntry[]>([])
  // Which option is selected in the Default-backend dropdown:
  //  'pub:<host>'  a public Electrum server (writes into settings.backend)
  //  'saved:<id>'  a saved backend (sets settings.default_backend)
  //  'custom'      the existing built-in connection, not a known public server
  let selectedKey = $state('custom')
  /// Toggles the TLS-cert-options + SOCKS5 cluster as one block at the
  /// bottom of the Connection card. Default collapsed — most users won't
  /// touch these. Opens automatically if the persisted config already has
  /// a non-default in any of these fields (so users don't lose their setup
  /// behind a closed disclosure).
  let showAdvanced = $state(false)
  // Network is set-once → collapsed; Default backend is the primary control →
  // expanded; the rest collapsed, each with a status hint.
  let showNetwork = $state(false)
  let showDefaultBackend = $state(true)
  let showPayjoin = $state(false)
  let showMempool = $state(false)
  let showNames = $state(false)
  let showBackends = $state(false)
  onDestroy(() => { if (flashTimer) clearTimeout(flashTimer) })

  // Diagnostics: a privacy-safe paste-into-a-bug-report blob. No seeds, xpubs,
  // addresses, labels, or amounts — only versions, connection health, and a
  // count of wallets by kind.
  let copiedDebug = $state(false)
  let copyDebugTimer: ReturnType<typeof setTimeout> | null = null
  onDestroy(() => { if (copyDebugTimer) clearTimeout(copyDebugTimer) })

  async function buildDebugInfo(): Promise<string> {
    let ver = { version: 'unknown', os: 'unknown', arch: 'unknown' }
    try { ver = await api.version() } catch { /* keep placeholders */ }
    const ns = get(nodeStatus)
    const bs = get(backendStatuses)
    const ws = get(wallets)

    const counts = new Map<string, number>()
    for (const w of ws) {
      const label = kindLabel(w.kind)
      counts.set(label, (counts.get(label) ?? 0) + 1)
    }
    const kindLines = counts.size
      ? [...counts].map(([k, n]) => `  ${k}: ${n}`).join('\n')
      : '  (none)'

    const backendLines = bs.length
      ? bs.map(b => `  - ${b.connected ? 'connected' : 'disconnected'}` +
          (b.tip_height != null ? `, tip ${b.tip_height}` : '') +
          (b.error ? `, error: ${b.error}` : '')).join('\n')
      : '  (none reported)'

    const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
    return [
      'Corvin debug info',
      `version: ${ver.version}`,
      `server os/arch: ${ver.os}/${ver.arch}`,
      `client: ${isTauri ? 'desktop (Tauri)' : 'browser'}`,
      `network: ${ns?.network ?? settings?.network.type ?? 'unknown'}`,
      `default backend connected: ${ns ? ns.connected : 'unknown'}` +
        (ns?.tip_height != null ? `, tip ${ns.tip_height}` : '') +
        (ns?.error ? `, error: ${ns.error}` : ''),
      'backends:',
      backendLines,
      `wallets: ${ws.length}`,
      kindLines,
    ].join('\n')
  }

  async function copyDebugInfo() {
    try {
      await navigator.clipboard.writeText(await buildDebugInfo())
      copiedDebug = true
      if (copyDebugTimer) clearTimeout(copyDebugTimer)
      copyDebugTimer = setTimeout(() => { copiedDebug = false }, 1800)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not copy debug info'
    }
  }

  // The built-in default isn't one of the known public servers — i.e. it's a
  // custom server (or a node) that should live in the saved-backends registry.
  function isCustomDefault(b: Settings['backend']): boolean {
    return b.type !== 'electrum' || !PUBLIC_SERVERS.some(s => s.host === b.electrum_host)
  }

  onMount(async () => {
    try {
      savedBackends = await api.backends.list().catch(() => [])
      let s = await api.settings.get()
      // Unify: if the default is a custom embedded connection, move it into the
      // registry (server-side, secret-safe) and pin it — so the default is always
      // a reference to a public server or a saved backend, never an ad-hoc one.
      if (!s.default_backend && isCustomDefault(s.backend)) {
        try {
          await api.backends.adoptDefault()
          s = await api.settings.get()
          savedBackends = await api.backends.list()
        } catch { /* leave as-is; selection falls back below */ }
      }
      settings = s
      // Resolve which option is currently selected.
      if (s.default_backend) {
        selectedKey = `saved:${s.default_backend}`
      } else {
        const match = s.backend.type === 'electrum'
          ? PUBLIC_SERVERS.find(sv => sv.host === s.backend.electrum_host)
          : undefined
        selectedKey = match ? `pub:${match.host}` : `pub:${PUBLIC_SERVERS[0].host}`
      }
      // SP + Mempool stay collapsed by default regardless of config — the
      // collapsed header shows a status hint, so nothing's hidden at a glance.
    } catch (e) { error = e instanceof Error ? e.message : 'Unknown error' }
  })

  function applyPublicServer(s: PublicServer) {
    if (!settings) return
    settings.backend.type = 'electrum'
    settings.backend.electrum_host = s.host
    settings.backend.electrum_port = s.port
    settings.backend.electrum_ssl = s.ssl
    settings.backend.validate_tls = true
    settings.backend.danger_accept_invalid_certs = false
  }

  function applySelection(key: string) {
    selectedKey = key
    testResult = null
    if (!settings) return
    if (key.startsWith('saved:')) {
      settings.default_backend = key.slice('saved:'.length)
    } else if (key.startsWith('pub:')) {
      const host = key.slice('pub:'.length)
      const found = PUBLIC_SERVERS.find(s => s.host === host)
      if (found) { settings.default_backend = null; applyPublicServer(found) }
    } else {
      // 'custom' — keep the existing built-in connection, just not via a pin.
      settings.default_backend = null
    }
    autosave('default')
  }

  // True when the default points at a saved backend (its connection is edited in
  // the Saved-backends section, not here).
  let usingSaved = $derived(!!settings?.default_backend)
  let selectedSavedBackend = $derived(savedBackends.find(b => b.id === settings?.default_backend) ?? null)
  // The default is for HD/regular wallets, so Frigate (SP-only) backends aren't
  // offered here — they're picked when creating a Silent Payments wallet.
  let defaultBackendOptions = $derived(savedBackends.filter(b => !b.frigate))
  // Collapsed-header hints.
  let networkLabel = $derived(
    ({ bitcoin: 'Mainnet', testnet: 'Testnet', signet: 'Signet', regtest: 'Regtest' } as Record<string, string>)[settings?.network.type ?? ''] ?? (settings?.network.type ?? '')
  )
  let defaultBackendHint = $derived(
    selectedSavedBackend
      ? selectedSavedBackend.label
      : (settings?.backend.type === 'rpc' ? settings.backend.rpc_url : (settings?.backend.electrum_host ?? ''))
  )

  let mempoolUrlInvalid = $derived.by((): string | null => {
    const v = settings?.backend.mempool_url?.trim() ?? ''
    if (v === '') return null // empty is fine — disables mempool features
    if (!/^https?:\/\//i.test(v)) return 'Must start with http:// or https://'
    try { new URL(v) } catch { return 'Not a valid URL' }
    return null
  })

  // Autosave: every control commits on change (toggles/dropdowns) or on blur
  // (text/number fields), so there's no global Save button. We deliberately do
  // NOT reassign `settings` from the response — that would clobber a value the
  // user is mid-editing in another field. The server preserves rpc_pass + the
  // backends registry, and the form never edits those, so the local state stays
  // a valid thing to re-PUT.
  async function autosave(section?: string) {
    if (!settings) return
    if (mempoolUrlInvalid) {
      error = `Mempool URL: ${mempoolUrlInvalid}`
      return
    }
    error = ''
    try {
      // Refresh the only fields this page doesn't own (the Display price flags)
      // from the latest config, so saving here never reverts a Display change.
      const fresh = await api.settings.get()
      settings.backend.show_price_data = fresh.backend.show_price_data
      settings.backend.show_current_price = fresh.backend.show_current_price
      settings.backend.show_fiat_balance = fresh.backend.show_fiat_balance
      await api.settings.update(settings)
      mempoolUrl.set(settings.backend.mempool_url)
      if (section) flash(section)
    } catch (e) { error = e instanceof Error ? e.message : 'Unknown error' }
  }

  async function testConnection() {
    if (!settings) return
    testResult = null
    try {
      // When the default is a saved backend, probe that registry entry (it holds
      // the real RPC password server-side); otherwise probe the built-in config.
      const s = selectedSavedBackend
        ? await api.backends.test(selectedSavedBackend)
        : await api.testStatus(settings)
      testResult = s.connected
        ? { ok: true, msg: `Connected · ${s.network}${s.tip_height ? ' · block ' + s.tip_height.toLocaleString() : ''}` }
        : { ok: false, msg: s.error ?? 'Connection failed' }
    } catch (e) {
      testResult = { ok: false, msg: e instanceof Error ? e.message : 'Unknown error' }
    }
  }

  async function testMempool() {
    if (!settings) return
    mempoolTestResult = null
    try {
      mempoolTestResult = await api.testMempool(settings.backend.mempool_url, settings.backend.socks5_proxy, settings.backend.mempool_danger_accept_invalid_certs)
    } catch (e) {
      mempoolTestResult = { ok: false, msg: e instanceof Error ? e.message : 'Unknown error' }
    }
  }
</script>

<div class="page">
  <div class="page-inner">
    <h1>Backend Settings <HelpLink anchor="connect-backend" /></h1>
    {#if error}<p class="global-error">{error}</p>{/if}

    {#if !settings}
      <p class="loading">Loading…</p>
    {:else}
      <form onsubmit={(e) => { e.preventDefault(); autosave() }}>

        <div class="card">
          <button type="button" class="card-toggle" onclick={() => showNetwork = !showNetwork} aria-expanded={showNetwork}>
            <span class="adv-arrow" class:open={showNetwork}>▸</span>
            <span class="card-title">Network</span>
            {#if flashed === 'network'}
              <span class="autosaved">Autosaved ✓</span>
            {:else if !showNetwork}
              <span class="card-toggle-hint">{networkLabel}</span>
            {/if}
          </button>
          {#if showNetwork}
            <div class="net-row">
              <select
                id="s-net"
                class="net-select"
                value={settings.network.type}
                onchange={(e) => {
                  if (!settings) return
                  const newKind = (e.target as HTMLSelectElement).value
                  const oldKind = settings.network.type
                  if (newKind !== oldKind && oldKind === 'bitcoin' && newKind !== 'bitcoin') {
                    if (!confirm('Switching from Mainnet to a test network will hide your existing wallets (they\'re only valid on Mainnet). They stay in storage and reappear if you switch back. Continue?')) {
                      return
                    }
                  }
                  if (newKind !== oldKind && oldKind !== 'bitcoin' && newKind === 'bitcoin') {
                    if (!confirm('Switching to Mainnet will hide any test-network wallets currently shown. Continue?')) {
                      return
                    }
                  }
                  settings.network.type = newKind as typeof settings.network.type
                  autosave('network')
                }}
              >
                <option value="bitcoin">Mainnet</option>
                <option value="testnet">Testnet</option>
                <option value="signet">Signet</option>
                <option value="regtest">Regtest</option>
              </select>
              <span class="net-hint">One network per instance. Restart required to fully apply.</span>
            </div>
            <label class="offline-row">
              <input
                type="checkbox"
                checked={settings.offline ?? false}
                onchange={async (e) => {
                  if (!settings) return
                  settings.offline = (e.target as HTMLInputElement).checked
                  await autosave('network')
                  // Refresh status so the offline indicator updates immediately.
                  try { nodeStatus.set(await api.status()) } catch { /* sidebar poll catches up */ }
                }}
              />
              <span>
                <strong>Offline mode</strong>
                <span class="net-hint">
                  Air-gapped use: Corvin never connects to a backend. You can add wallets,
                  view cached balances, and import + sign + export PSBTs. Sync and broadcast
                  are unavailable. (True air-gap also means keeping this machine offline.)
                </span>
              </span>
            </label>
          {/if}
        </div>

        <div class="card">
          <button type="button" class="card-toggle" onclick={() => showDefaultBackend = !showDefaultBackend} aria-expanded={showDefaultBackend}>
            <span class="adv-arrow" class:open={showDefaultBackend}>▸</span>
            <span class="card-title">Default backend</span>
            {#if flashed === 'default'}
              <span class="autosaved">Autosaved ✓</span>
            {:else if !showDefaultBackend}
              <span class="card-toggle-hint">{defaultBackendHint}</span>
            {/if}
          </button>

          {#if showDefaultBackend}
          <div class="row">
            <div class="field" style="flex:1">
              <label for="s-default">Server</label>
              <select id="s-default" value={selectedKey} onchange={(e) => applySelection((e.target as HTMLSelectElement).value)}>
                <optgroup label="Public Electrum servers">
                  {#each PUBLIC_SERVERS as s (s.host)}
                    <option value={`pub:${s.host}`}>{s.host}</option>
                  {/each}
                </optgroup>
                {#if defaultBackendOptions.length}
                  <optgroup label="Your saved backends">
                    {#each defaultBackendOptions as b (b.id)}
                      <option value={`saved:${b.id}`}>{b.label}</option>
                    {/each}
                  </optgroup>
                {/if}
              </select>
            </div>
            <div class="field w-sync">
              <label for="s-sync">Auto-sync (s)</label>
              <input id="s-sync" type="number" min="10" max="3600" bind:value={settings.backend.poll_interval_secs} onchange={() => autosave('default')} />
            </div>
          </div>

          <p class="conn-note">
            {#if usingSaved}
              Wallets without their own backend use your saved <strong>{selectedSavedBackend?.label ?? 'backend'}</strong>. Edit its connection in <button type="button" class="link-btn" onclick={() => { showBackends = true; setTimeout(() => document.getElementById('backends-section')?.scrollIntoView({ behavior: 'smooth' }), 0) }}>Saved backends</button> below.
            {:else}
              Wallets without their own backend use this one. Add private Electrum servers or your own node under <button type="button" class="link-btn" onclick={() => { showBackends = true; setTimeout(() => document.getElementById('backends-section')?.scrollIntoView({ behavior: 'smooth' }), 0) }}>Saved backends</button>, then pick one here or per wallet.
            {/if}
          </p>

          <div class="test-row">
            <BusyButton variant="secondary" bind:busy={testing} idle="Test connection" busyLabel="Testing…" onclick={testConnection} />
            {#if testResult}
              <span class="test-result" class:ok={testResult.ok} class:fail={!testResult.ok}>
                {testResult.ok ? '✓' : '✗'} {testResult.msg}
              </span>
            {/if}
          </div>

          <button
            type="button"
            class="advanced-toggle"
            onclick={() => showAdvanced = !showAdvanced}
            aria-expanded={showAdvanced}
          >
            <span class="adv-arrow" class:open={showAdvanced}>▸</span>
            Advanced (Tor proxy)
          </button>

          {#if showAdvanced}
            <div class="advanced-block">
              <div class="check">
                <input
                  id="c-proxy"
                  type="checkbox"
                  checked={!!settings.backend.socks5_proxy}
                  onchange={(e) => {
                    if (!settings) return
                    settings.backend.socks5_proxy = (e.target as HTMLInputElement).checked ? '127.0.0.1:9050' : null
                    autosave('default')
                  }}
                />
                <label for="c-proxy">Route through SOCKS5 proxy (Tor)</label>
              </div>
              {#if settings.backend.socks5_proxy !== null && settings.backend.socks5_proxy !== undefined}
                <div class="field">
                  <label for="s-proxy">Proxy address</label>
                  <input
                    id="s-proxy"
                    type="text"
                    placeholder="127.0.0.1:9050"
                    value={settings.backend.socks5_proxy ?? ''}
                    oninput={(e) => {
                      if (!settings) return
                      const v = (e.target as HTMLInputElement).value.trim()
                      settings.backend.socks5_proxy = v || null
                    }}
                    onchange={() => autosave('default')}
                  />
                </div>
              {/if}
            </div>
          {/if}
          {/if}
        </div>

        <div class="card" id="backends-section">
          <button type="button" class="card-toggle" onclick={() => showBackends = !showBackends} aria-expanded={showBackends}>
            <span class="adv-arrow" class:open={showBackends}>▸</span>
            <span class="card-title">Saved backends</span>
            {#if !showBackends}
              <span class="card-toggle-hint">Per-wallet servers</span>
            {/if}
          </button>
          {#if showBackends}
            <BackendsSection defaultBackendId={settings.default_backend} />
          {/if}
        </div>

        <div class="card">
          <button type="button" class="card-toggle" onclick={() => showMempool = !showMempool} aria-expanded={showMempool}>
            <span class="adv-arrow" class:open={showMempool}>▸</span>
            <span class="card-title">Mempool</span>
            {#if flashed === 'mempool'}
              <span class="autosaved">Autosaved ✓</span>
            {:else if !showMempool}
              <span class="card-toggle-hint">{settings.backend.mempool_url?.replace(/^https?:\/\//, '') || 'disabled'}</span>
            {/if}
          </button>

          {#if showMempool}
          <p class="card-desc">
            Used for fee rate estimates and transaction links. Point at your own instance or leave as mempool.space.
            {#if settings.backend.socks5_proxy}
              <strong>Tor tip:</strong> use mempool.space's onion address
              (<code class="inline-code">mempoolhqx4isw62xs7abwphsq7ldayuidyx2v2oethdhhj6mlo2r6ad.onion</code>)
              for end-to-end Tor routing instead of exit-node TLS.
            {/if}
          </p>
          <div class="field">
            <label for="s-mempool">Server URL</label>
            <input
              id="s-mempool"
              type="text"
              bind:value={settings.backend.mempool_url}
              placeholder="https://mempool.space"
              class:invalid={mempoolUrlInvalid !== null}
              onchange={() => autosave('mempool')}
            />
            {#if mempoolUrlInvalid}
              <span class="hint warn">{mempoolUrlInvalid}</span>
            {/if}
          </div>
          {#if settings.backend.mempool_url.toLowerCase().startsWith('https://')}
            <div class="check">
              <input id="c-mempool-invalid" type="checkbox" bind:checked={settings.backend.mempool_danger_accept_invalid_certs} onchange={() => autosave('mempool')} />
              <label for="c-mempool-invalid">
                Accept invalid / self-signed certificate
                <span class="hint warn">— for a self-hosted mempool server with its own cert</span>
              </label>
            </div>
          {/if}
          <div class="test-row">
            <BusyButton variant="secondary" bind:busy={mempoolTesting} idle="Test connection" busyLabel="Testing…" onclick={testMempool} />
            {#if mempoolTestResult}
              <span class="test-result" class:ok={mempoolTestResult.ok} class:fail={!mempoolTestResult.ok}>
                {mempoolTestResult.ok ? '✓' : '✗'} {mempoolTestResult.msg}
              </span>
            {/if}
          </div>
          {/if}
        </div>

        <div class="card">
          <button type="button" class="card-toggle" onclick={() => showPayjoin = !showPayjoin} aria-expanded={showPayjoin}>
            <span class="adv-arrow" class:open={showPayjoin}>▸</span>
            <span class="card-title">Payjoin</span>
            {#if flashed === 'payjoin'}
              <span class="autosaved">Autosaved ✓</span>
            {:else if !showPayjoin}
              <span class="card-toggle-hint">{settings.backend.payjoin_enabled ? 'Enabled' : 'Off'}</span>
            {/if}
          </button>

          {#if showPayjoin}
            <div class="check">
              <input id="c-pj-enabled" type="checkbox" bind:checked={settings.backend.payjoin_enabled} onchange={() => autosave('payjoin')} />
              <label for="c-pj-enabled">Enable Payjoin (BIP-77 / v2)</label>
            </div>
            <p class="sp-status">
              Opts in to coordinating payments privately via a public directory + OHTTP relay (uses the same SOCKS5 proxy as your backend); the directory sees encrypted blobs, not your IP. <strong>Sending</strong> works from any software single-sig wallet. <strong>Receiving</strong> additionally needs a wallet whose backend is a Bitcoin node (RPC) — it's offered per wallet, not globally.
            </p>
            {#if settings.backend.payjoin_enabled}
              <div class="field">
                <label for="pj-dir">Payjoin directory</label>
                <input id="pj-dir" type="text" placeholder="https://payjo.in" bind:value={settings.backend.payjoin_directory_url} onchange={() => autosave('payjoin')} />
              </div>
              <div class="field">
                <label for="pj-relay">OHTTP relay</label>
                <input id="pj-relay" type="text" placeholder="https://pj.bobspacebkk.com" bind:value={settings.backend.payjoin_ohttp_relay_url} onchange={() => autosave('payjoin')} />
              </div>
              <div class="field">
                <label for="pj-fallback">Send original if no response after (seconds)</label>
                <input id="pj-fallback" type="number" min="30" max="3600" bind:value={settings.backend.payjoin_fallback_secs} onchange={() => autosave('payjoin')} />
              </div>
            {/if}
          {/if}
        </div>

        <div class="card">
          <button type="button" class="card-toggle" onclick={() => showNames = !showNames} aria-expanded={showNames}>
            <span class="adv-arrow" class:open={showNames}>▸</span>
            <span class="card-title">Name resolution (BIP-353)</span>
            {#if flashed === 'names'}
              <span class="autosaved">Autosaved ✓</span>
            {:else if !showNames}
              <span class="card-toggle-hint">{settings.backend.bip353_doh_url?.replace(/^https?:\/\//, '').replace(/\/.*$/, '') || 'cloudflare-dns.com'}</span>
            {/if}
          </button>

          {#if showNames}
          <p class="card-desc">
            Paying a <code class="inline-code">₿user@domain</code> name resolves it over DNS-over-HTTPS. DNSSEC prevents a forged answer, but the resolver sees which name you look up, which is who you're about to pay.
            {#if settings.backend.socks5_proxy}
              Your SOCKS5 proxy hides your IP from the resolver, not the name.
            {/if}
          </p>
          <div class="field">
            <label for="s-doh">DoH resolver URL</label>
            <input
              id="s-doh"
              type="text"
              bind:value={settings.backend.bip353_doh_url}
              placeholder="https://cloudflare-dns.com/dns-query"
              onchange={() => autosave('names')}
            />
          </div>
          {/if}
        </div>

      </form>
    {/if}

    <div class="card diag-card">
      <div class="diag-head">
        <span class="card-title">Diagnostics</span>
        <button type="button" class="btn-secondary" onclick={copyDebugInfo}>
          {copiedDebug ? 'Copied ✓' : 'Copy debug info'}
        </button>
      </div>
      <p class="conn-note">
        Copies version, connection status, and a count of wallets by kind for bug
        reports. No seeds, keys, addresses, labels, or balances are included.
      </p>
    </div>
  </div>
</div>

<style>
  .page {
    flex: 1; overflow-y: auto; background: var(--surface-2);
    padding: 32px 24px 48px;
  }
  .page-inner { max-width: 640px; margin: 0 auto; }
  h1 { font-size: 1.4rem; font-weight: 700; color: var(--text); margin: 0 0 24px; letter-spacing: -0.01em; }
  .global-error { color: var(--error); font-size: 0.82rem; margin: 0 0 12px; }
  .loading { color: var(--text-muted); font-size: 0.85rem; margin: 0; }

  form { display: contents; }

  .card {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px;
    padding: 20px 24px; display: flex; flex-direction: column; gap: 14px;
    margin-bottom: 12px;
  }

  /* Compact inline selector for the Network card — the save flow handles the
     actual confirm prompt for cross-network switches. */
  .net-row {
    display: flex; align-items: center; gap: 10px; flex-wrap: wrap;
  }
  .net-select {
    width: auto; min-width: 130px; padding: 5px 8px; font-size: 0.82rem;
  }
  .net-hint { font-size: 0.72rem; color: var(--text-muted); }
  .diag-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .offline-row {
    display: flex; align-items: flex-start; gap: 10px; margin-top: 14px; cursor: pointer;
  }
  .offline-row input { margin-top: 2px; flex-shrink: 0; }
  .offline-row > span { display: flex; flex-direction: column; gap: 3px; }
  .offline-row strong { font-size: 0.85rem; color: var(--text); }
  .conn-note { font-size: 0.78rem; color: var(--text-muted); margin: 8px 0 0; line-height: 1.5; }
  .link-btn {
    background: none; border: none; padding: 0; cursor: pointer;
    color: var(--accent); font-size: inherit; text-decoration: underline;
  }

  /* Disclosure for the SOCKS5/Tor proxy option. Collapsed by default. */
  .advanced-toggle {
    align-self: flex-start;
    background: none; border: none; padding: 4px 0; cursor: pointer;
    color: var(--text-muted); font-size: 0.8rem;
    display: inline-flex; align-items: center; gap: 6px;
  }
  .advanced-toggle:hover { color: var(--text); }
  .adv-arrow { transition: transform 0.1s; display: inline-block; }
  .adv-arrow.open { transform: rotate(90deg); }
  .advanced-block {
    display: flex; flex-direction: column; gap: 10px;
    padding: 12px; margin-top: 2px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
  }

  /* SP scanner status line + inline action — collapsed default. */
  .sp-status {
    margin: 0; font-size: 0.85rem; color: var(--text-muted); line-height: 1.5;
  }
  .card-title { font-size: 0.9rem; font-weight: 600; color: var(--text); }
  /* Collapsible section header — every card uses this now. */
  .card-toggle {
    display: flex; align-items: center; gap: 8px; width: 100%;
    background: none; border: none; padding: 0; cursor: pointer; text-align: left;
  }
  .card-toggle-hint {
    margin-left: auto; font-size: 0.78rem; color: var(--text-muted);
    font-family: var(--font-mono); white-space: nowrap; overflow: hidden;
    text-overflow: ellipsis; max-width: 55%;
  }
  .card-desc { font-size: 0.82rem; color: var(--text-muted); margin: 0; line-height: 1.5; }

  .row { display: flex; gap: 12px; align-items: flex-end; }
  .field { display: flex; flex-direction: column; gap: 5px; }
  .w-sync { width: 130px; flex-shrink: 0; }

  label { font-size: 0.8rem; color: var(--text-muted); display: block; }
  input, select {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%; display: block;
  }
  input:focus, select:focus { outline: 1px solid var(--accent); outline-offset: -1px; }
  input[type="number"] { appearance: textfield; -moz-appearance: textfield; }

  .hint { font-size: 0.72rem; color: var(--text-muted); }
  .hint.warn { color: #e09c52; }
  input.invalid { border-color: #e09c52; }
  .inline-code { font-family: monospace; font-size: 0.72rem; background: var(--surface-2); padding: 1px 4px; border-radius: 3px; word-break: break-all; }

  .check { display: flex; align-items: baseline; gap: 8px; }
  .check input[type="checkbox"] { width: auto; margin: 0; flex-shrink: 0; cursor: pointer; }
  .check label { font-size: 0.85rem; color: var(--text); cursor: pointer; display: inline; }
  .warn { color: #e09c52; font-size: 0.78rem; }

  .test-row { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .test-result { font-size: 0.8rem; }
  .test-result.ok { color: #52a875; }
  .test-result.fail { color: var(--error); }

  /* Transient "Autosaved ✓" indicator in a card header, right-aligned like the
     status hint it replaces while showing. */
  .autosaved {
    margin-left: auto; font-size: 0.72rem; font-weight: 500; color: #52a875;
    white-space: nowrap;
  }
  @media (max-width: 768px) {
    .page { padding: 16px 16px 32px; }
    h1 { display: none; }
    .row { flex-wrap: wrap; }
    .w-sync { width: auto; flex: 1; }
  }
</style>
