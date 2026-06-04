<script lang="ts">
  import { goto } from '$app/navigation'
  import { api } from '../lib/api'
  import { securityState } from '../stores/security'
  import { addToast } from '../stores/toasts'

  let { onClose }: { onClose: () => void } = $props()

  // 1 = welcome, 2 = how you connect, 3 = protect your data, 4 = add wallet handoff.
  let step = $state(1)
  let network = $state('bitcoin')
  let savingNetwork = $state(false)

  // Step 3: optional at-rest encryption, set up before any wallet exists so no
  // plaintext wallet data is ever written.
  let password = $state('')
  let confirm = $state('')
  let enabling = $state(false)
  let canEnable = $derived(password.length >= 8 && password === confirm)

  async function enableAndContinue() {
    if (!canEnable || enabling) return
    enabling = true
    try {
      await api.security.enable(password)
      securityState.set('unlocked')
      password = ''
      confirm = ''
      step = 4
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Could not enable encryption')
    } finally {
      enabling = false
    }
  }

  const NETWORKS = [
    { value: 'bitcoin', label: 'Mainnet' },
    { value: 'testnet', label: 'Testnet' },
    { value: 'signet', label: 'Signet' },
    { value: 'regtest', label: 'Regtest' },
  ]

  // Persist the chosen network if it isn't the default. Best-effort; we re-fetch
  // settings so we only send a changed network and don't clobber other fields.
  async function saveNetwork() {
    if (network === 'bitcoin') return
    savingNetwork = true
    try {
      const s = await api.settings.get()
      if (s.network.type !== network) {
        s.network.type = network
        await api.settings.update(s)
      }
    } catch { /* non-fatal; they can set it in Backend settings */ }
    finally { savingNetwork = false }
  }

  async function done(navigateToAddWallet: boolean) {
    await saveNetwork()
    api.completeOnboarding().catch(() => {})
    onClose()
    if (navigateToAddWallet) goto('/add-wallet')
  }

  function skip() {
    api.completeOnboarding().catch(() => {})
    onClose()
  }

  function openBackendSettings() {
    // Heading into manual setup, so onboarding's job is done.
    api.completeOnboarding().catch(() => {})
    onClose()
    goto('/settings/backend')
  }
</script>

<!-- Escape must not skip-complete onboarding from the encryption step (3), where it
     would silently discard a half-typed password and the one-time encryption prompt. -->
<svelte:window onkeydown={(e) => { if (e.key === 'Escape' && step !== 3) skip() }} />

<div class="overlay" role="dialog" aria-modal="true" aria-labelledby="ob-title">
  <div class="card">
    <button class="skip" onclick={skip}>Skip</button>

    {#if step === 1}
      <div class="welcome-head">
        <div class="btc-mark" aria-hidden="true">₿</div>
        <div>
          <h1 id="ob-title">Welcome to Corvin</h1>
          <p class="lede welcome-lede">A Bitcoin wallet under your complete control.</p>
        </div>
      </div>
      <ul class="points">
        <li><strong>Your keys.</strong> You hold the recovery phrase. No custodian, no account.</li>
        <li><strong>Open source.</strong> Anyone can read and audit the code.</li>
        <li><strong>You choose the backend.</strong> Start on a public one, or use your own node for privacy.</li>
      </ul>
      <div class="actions">
        <!-- svelte-ignore a11y_autofocus -->
        <button class="btn-primary" autofocus onclick={() => (step = 2)}>Get started</button>
      </div>
    {:else if step === 2}
      <h1 id="ob-title">How Corvin connects</h1>
      <p class="lede">Corvin reaches the Bitcoin network through a backend you choose. It already works out of the box, and you can change any of this later in Backend settings.</p>

      <div class="field">
        <label for="ob-net">Network</label>
        <select id="ob-net" bind:value={network}>
          {#each NETWORKS as n (n.value)}
            <option value={n.value}>{n.label}</option>
          {/each}
        </select>
        <p class="hint">Most people want Mainnet. One network per instance. Pick a test network only if you know you need one.</p>
      </div>

      <div class="info">
        <strong>Backend:</strong> you're on a public Electrum server, so Corvin works right now. For more privacy, point it at your own node or Electrum server. Whoever runs a backend can see the addresses you watch.
      </div>

      <div class="actions">
        <button class="btn-secondary back-btn" onclick={() => (step = 1)}>Back</button>
        <button class="btn-ghost" onclick={openBackendSettings}>Connect my own backend</button>
        <button class="btn-primary" onclick={() => (step = 3)} disabled={savingNetwork}>Continue</button>
      </div>
    {:else if step === 3}
      <h1 id="ob-title">Protect your data</h1>
      <p class="lede">Optionally encrypt everything Corvin stores on disk (wallet data, labels, settings) with a password. Doing it now means no unencrypted wallet data is ever written. Your seed is never stored either way, so your funds are always safe. You can also turn this on later in Security settings.</p>
      <div class="info warn">
        <strong>Choose this password carefully.</strong> There is no recovery. If you forget it you lose your labels and settings (re-add wallets from your seed to recover the money). Corvin never stores it.
      </div>
      <div class="field">
        <label for="ob-pw">Password</label>
        <input id="ob-pw" type="password" autocomplete="new-password" bind:value={password} placeholder="At least 8 characters" disabled={enabling} />
      </div>
      <div class="field">
        <label for="ob-pw2">Confirm password</label>
        <input id="ob-pw2" type="password" autocomplete="new-password" bind:value={confirm} placeholder="Re-enter password" disabled={enabling} />
        {#if confirm.length > 0 && password !== confirm}
          <p class="hint warn-text">Passwords don't match.</p>
        {/if}
      </div>
      <div class="actions">
        <button class="btn-secondary back-btn" onclick={() => (step = 2)}>Back</button>
        <button class="btn-ghost" onclick={() => (step = 4)} disabled={enabling}>Skip for now</button>
        <button class="btn-primary" onclick={enableAndContinue} disabled={!canEnable || enabling}>
          {enabling ? 'Encrypting…' : 'Encrypt and continue'}
        </button>
      </div>
    {:else}
      <h1 id="ob-title">Add your first wallet</h1>
      <p class="lede">Create a new wallet, or import one you already have from a seed, an xpub, a descriptor, or a hardware wallet.</p>
      <div class="info warn">
        <strong>Back up your recovery phrase.</strong> If you create a new wallet, write the phrase down and keep it somewhere safe offline. It's the only way to restore your coins, and Corvin never stores it for you.
      </div>
      <div class="actions">
        <button class="btn-secondary back-btn" onclick={() => (step = 3)}>Back</button>
        <button class="btn-primary" onclick={() => done(true)} disabled={savingNetwork}>Add a wallet</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .overlay {
    position: fixed; inset: 0; z-index: 50;
    background: var(--surface-2);
    display: flex; align-items: center; justify-content: center;
    padding: 24px;
  }
  .card {
    position: relative;
    width: min(520px, 100%);
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 32px 32px 26px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  }
  .skip {
    position: absolute; top: 14px; right: 16px;
    background: none; border: none; color: var(--text-muted);
    font-size: 0.82rem; cursor: pointer; padding: 4px 6px;
  }
  .skip:hover { color: var(--text); }

  .welcome-head {
    display: flex; align-items: center; gap: 16px; margin-bottom: 22px;
  }
  .btc-mark {
    font-size: 3rem; font-weight: 800; line-height: 1;
    color: var(--accent); flex-shrink: 0;
  }
  /* In the welcome lockup the title + subtitle stack tightly beside the mark. */
  .welcome-head h1 { margin: 0 0 4px; }
  .welcome-lede { margin: 0; }
  h1 { font-size: 1.4rem; font-weight: 700; color: var(--text); margin: 0 0 14px; letter-spacing: -0.01em; }
  .lede { font-size: 0.92rem; line-height: 1.6; color: var(--text-muted); margin: 0 0 18px; }

  .points { list-style: none; margin: 0 0 22px; padding: 0; display: flex; flex-direction: column; gap: 12px; }
  .points li { font-size: 0.92rem; line-height: 1.55; color: var(--text-muted); }
  .points strong { color: var(--text); font-weight: 600; }

  .field { margin-bottom: 16px; }
  .field label {
    display: block; font-size: 0.74rem; font-weight: 600; color: var(--text-muted);
    margin-bottom: 6px; text-transform: uppercase; letter-spacing: 0.06em;
  }
  .field select {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; color: var(--text); padding: 8px 10px; font-size: 0.88rem; outline: none;
  }
  .field select:focus { border-color: var(--accent); }
  .field input {
    width: 100%; box-sizing: border-box;
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 5px; color: var(--text); padding: 8px 10px; font-size: 0.9rem; outline: none;
  }
  .field input:focus { border-color: var(--accent); }
  .hint { font-size: 0.76rem; color: var(--text-muted); margin: 6px 0 0; line-height: 1.5; }
  .warn-text { color: var(--danger, #d9684a); }

  .info {
    font-size: 0.85rem; line-height: 1.55; color: var(--text-muted);
    background: var(--surface-2); border: 1px solid var(--border);
    border-radius: 7px; padding: 12px 14px; margin-bottom: 20px;
  }
  .info strong { color: var(--text); font-weight: 600; }
  .info.warn {
    background: color-mix(in srgb, #e09c52 12%, var(--surface-2));
    border-color: color-mix(in srgb, #e09c52 35%, var(--border));
  }

  .actions { display: flex; align-items: center; gap: 10px; justify-content: flex-end; }
  /* The Back button sits at the far left; the rest cluster right. */
  .back-btn { margin-right: auto; }

  @media (max-width: 600px) {
    .card { padding: 26px 20px 22px; }
    .actions { flex-wrap: wrap; }
    .back-btn { margin-right: 0; }
  }
</style>
