<script lang="ts">
  import { onMount } from 'svelte'
  import { api } from '../lib/api'
  import { securityState } from '../stores/security'
  import { addToast } from '../stores/toasts'
  import { passwordStrength } from '../lib/password'
  import Modal from './ui/Modal.svelte'

  let loading = $state(true)
  let busy = $state(false)
  let showDisableConfirm = $state(false)
  let disablePassword = $state('')
  let disableError = $state('')

  // Enable form.
  let password = $state('')
  let confirm = $state('')

  let canEnable = $derived(password.length >= 8 && password === confirm)
  let pwStrength = $derived(passwordStrength(password))

  // Change-password form (unlocked state).
  let cpCurrent = $state('')
  let cpNew = $state('')
  let cpConfirm = $state('')
  let cpBusy = $state(false)
  let cpError = $state('')
  let canChange = $derived(cpCurrent.length > 0 && cpNew.length >= 8 && cpNew === cpConfirm)
  let cpStrength = $derived(passwordStrength(cpNew))

  onMount(async () => {
    try {
      securityState.set((await api.security.status()).state)
    } catch {
      // leave as-is
    } finally {
      loading = false
    }
  })

  async function enable() {
    if (!canEnable || busy) return
    busy = true
    try {
      const r = await api.security.enable(password)
      securityState.set(r.state)
      password = ''
      confirm = ''
      addToast('Wallet data is now encrypted')
    } catch (e) {
      addToast(e instanceof Error ? e.message : 'Could not enable encryption')
    } finally {
      busy = false
    }
  }

  async function changePassword() {
    if (!canChange || cpBusy) return
    cpBusy = true
    cpError = ''
    try {
      await api.security.changePassword(cpCurrent, cpNew)
      cpCurrent = ''
      cpNew = ''
      cpConfirm = ''
      addToast('Password changed')
    } catch (e) {
      const code = (e as { code?: string })?.code
      cpError = code === 'wrong_secret'
        ? 'Current password is incorrect.'
        : e instanceof Error ? e.message : 'Could not change password'
    } finally {
      cpBusy = false
    }
  }

  function openDisable() {
    disablePassword = ''
    disableError = ''
    showDisableConfirm = true
  }

  async function disable() {
    if (busy || !disablePassword) return
    busy = true
    disableError = ''
    try {
      const r = await api.security.disable(disablePassword)
      securityState.set(r.state)
      showDisableConfirm = false
      disablePassword = ''
      addToast('Encryption disabled')
    } catch (e) {
      const code = (e as { code?: string })?.code
      if (code === 'wrong_secret') {
        disableError = 'Incorrect password.'
      } else {
        disableError = e instanceof Error ? e.message : 'Could not disable encryption'
      }
    } finally {
      busy = false
    }
  }
</script>

<div class="page">
  <div class="page-inner">
    <h1>Security Settings</h1>

    {#if loading}
      <p class="loading">Loading…</p>
    {:else if $securityState === 'unlocked'}
      <div class="card">
        <div class="card-head">
          <span class="lock" aria-hidden="true">🔒</span>
          <span class="card-title">Your data is encrypted</span>
        </div>
        <p class="card-desc">
          Corvin encrypts everything it stores on disk (wallet data, labels, settings) with your
          password. Your seed is never stored either way, so your funds are always safe. This
          protects the privacy of a stolen or copied config folder.
        </p>
        <p class="card-desc">
          After a restart, Corvin boots locked and asks for your password before loading anything.
          Background sync and notifications pause until you unlock.
        </p>
        <button class="btn-danger" disabled={busy} onclick={openDisable}>
          Disable encryption
        </button>
      </div>

      <div class="card">
        <div class="card-head">
          <span class="card-title">Change password</span>
        </div>
        <p class="card-desc">
          Set a new password. This re-encrypts everything under the new key in one pass, so your data
          is never written to disk unencrypted. Your current password is required.
        </p>
        <div class="field">
          <label for="cp-current">Current password</label>
          <input id="cp-current" type="password" autocomplete="current-password"
            bind:value={cpCurrent} placeholder="Current password" disabled={cpBusy} />
        </div>
        <div class="field">
          <label for="cp-new">New password</label>
          <input id="cp-new" type="password" autocomplete="new-password"
            bind:value={cpNew} placeholder="At least 8 characters" disabled={cpBusy} />
          {#if cpNew}<span class="hint strength s{cpStrength.score}">Strength: {cpStrength.label}</span>{/if}
        </div>
        <div class="field">
          <label for="cp-confirm">Confirm new password</label>
          <input id="cp-confirm" type="password" autocomplete="new-password"
            bind:value={cpConfirm} placeholder="Re-enter new password" disabled={cpBusy} />
          {#if cpConfirm.length > 0 && cpNew !== cpConfirm}
            <span class="hint warn-text">Passwords don't match.</span>
          {/if}
        </div>
        {#if cpError}<span class="hint warn-text">{cpError}</span>{/if}
        <button class="btn-primary" disabled={!canChange || cpBusy} onclick={changePassword}>
          {cpBusy ? 'Changing…' : 'Change password'}
        </button>
      </div>
    {:else}
      <div class="card">
        <div class="card-head">
          <span class="lock" aria-hidden="true">🔓</span>
          <span class="card-title">Encryption is off</span>
        </div>
        <p class="card-desc">
          Optionally encrypt everything Corvin stores on disk (wallet data, labels, settings) with a
          password. Your seed is never stored either way, so your funds are always safe. This
          protects the privacy of a stolen or copied config folder.
        </p>
        <div class="warn">
          <strong>Choose this password carefully.</strong> There is no recovery. If you forget it you
          lose your labels, notes, and saved settings (re-add wallets from your seed or xpub to recover
          the money). Corvin never stores the password.
        </div>
        <div class="field">
          <label for="pw">Password</label>
          <input id="pw" type="password" autocomplete="new-password" bind:value={password}
            placeholder="At least 8 characters" disabled={busy} />
          {#if password}<span class="hint strength s{pwStrength.score}">Strength: {pwStrength.label}</span>{/if}
        </div>
        <div class="field">
          <label for="pw2">Confirm password</label>
          <input id="pw2" type="password" autocomplete="new-password" bind:value={confirm}
            placeholder="Re-enter password" disabled={busy} />
          {#if confirm.length > 0 && password !== confirm}
            <span class="hint warn-text">Passwords don't match.</span>
          {/if}
        </div>
        <button class="btn-primary" disabled={!canEnable || busy} onclick={enable}>
          {busy ? 'Encrypting…' : 'Encrypt wallet data'}
        </button>
      </div>
    {/if}
  </div>
</div>

<Modal bind:open={showDisableConfirm} title="Disable encryption?" width="440px">
  <p class="modal-text">
    This writes all your wallet data back to disk unencrypted. Your seed is never stored either way,
    so your funds stay safe, but the privacy of a stolen or copied config folder is no longer
    protected. You can re-enable encryption later.
  </p>
  <div class="field">
    <label for="disable-pw">Confirm your password to continue</label>
    <input id="disable-pw" type="password" autocomplete="current-password"
      bind:value={disablePassword} placeholder="Password" disabled={busy}
      onkeydown={(e) => { if (e.key === 'Enter') disable() }} />
    {#if disableError}<span class="hint warn-text">{disableError}</span>{/if}
  </div>
  {#snippet footer()}
    <button class="btn-secondary" disabled={busy} onclick={() => (showDisableConfirm = false)}>
      Cancel
    </button>
    <button class="btn-danger" disabled={busy || !disablePassword} onclick={disable}>
      {busy ? 'Disabling…' : 'Disable encryption'}
    </button>
  {/snippet}
</Modal>

<style>
  .page {
    flex: 1; overflow-y: auto; background: var(--surface-2);
    padding: 32px 24px 48px;
  }
  .page-inner { max-width: 640px; margin: 0 auto; }
  h1 { font-size: 1.4rem; font-weight: 700; color: var(--text); margin: 0 0 24px; letter-spacing: -0.01em; }
  .loading { color: var(--text-muted); font-size: 0.88rem; }

  .card {
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 8px;
    padding: 20px 24px; display: flex; flex-direction: column; gap: 14px;
  }
  .card-head { display: flex; align-items: center; gap: 8px; }
  .card-title { font-size: 0.9rem; font-weight: 600; color: var(--text); }
  .lock { font-variant-emoji: text; color: var(--text-muted); }
  .card-desc { font-size: 0.82rem; color: var(--text-muted); margin: 0; line-height: 1.5; }

  .warn {
    background: color-mix(in srgb, #e09c52 12%, var(--surface-2));
    border: 1px solid color-mix(in srgb, #e09c52 35%, var(--border));
    border-radius: 6px; padding: 10px 12px; font-size: 0.82rem; line-height: 1.5;
  }
  .warn strong { color: var(--text); }

  .field { display: flex; flex-direction: column; gap: 5px; }
  label { font-size: 0.8rem; color: var(--text-muted); display: block; }
  input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 8px 10px; font-size: 0.85rem; width: 100%; display: block;
    box-sizing: border-box;
  }
  input:focus { outline: 1px solid var(--accent); outline-offset: -1px; }

  .hint { font-size: 0.72rem; color: var(--text-muted); }
  .warn-text { color: #e09c52; }
  .strength.s1 { color: #e05a4f; }
  .strength.s2 { color: #e09c52; }
  .strength.s3 { color: #b6b85a; }
  .strength.s4 { color: #4fbf67; }
  .card button { align-self: flex-start; }
  .modal-text { font-size: 0.85rem; color: var(--text-muted); margin: 0; line-height: 1.5; }
</style>
