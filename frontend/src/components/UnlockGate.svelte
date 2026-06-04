<script lang="ts">
  // Full-screen gate shown while at-rest encryption is on and not yet unlocked.
  // The rest of the app is not rendered until the password unlocks the vault.
  import { api } from '../lib/api'

  let { onUnlocked }: { onUnlocked: () => void } = $props()

  let password = $state('')
  let busy = $state(false)
  let error = $state('')
  let failures = $state(0)

  async function submit(e: Event) {
    e.preventDefault()
    if (!password || busy) return
    busy = true
    error = ''
    try {
      await api.security.unlock(password)
      password = ''
      onUnlocked()
    } catch (err) {
      failures += 1
      error = err instanceof Error ? err.message : 'Unlock failed'
    } finally {
      busy = false
    }
  }
</script>

<div class="gate">
  <form class="card" onsubmit={submit}>
    <div class="lock" aria-hidden="true">🔒</div>
    <h1>Corvin is locked</h1>
    <p class="sub">Enter your password to unlock this wallet.</p>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="password"
      bind:value={password}
      placeholder="Password"
      autocomplete="current-password"
      autofocus
      disabled={busy}
    />
    {#if error}
      <p class="err" role="alert">{error}</p>
    {/if}
    {#if failures >= 3}
      <p class="recovery" role="note">
        There is no password recovery. If you've forgotten it, your money is still safe: delete
        Corvin's config folder and re-add each wallet from its recovery phrase or xpub. You'll lose
        labels, notes, and settings, never the coins.
      </p>
    {/if}
    <button class="btn-primary" type="submit" disabled={busy || !password}>
      {busy ? 'Unlocking…' : 'Unlock'}
    </button>
  </form>
</div>

<style>
  .gate {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg);
    padding: 24px;
  }
  .card {
    width: 100%;
    max-width: 360px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    text-align: center;
  }
  .lock {
    font-size: 2.2rem;
    font-variant-emoji: text;
    color: var(--text-muted);
  }
  h1 {
    margin: 0;
    font-size: 1.3rem;
  }
  .sub {
    margin: 0;
    color: var(--text-muted);
    font-size: 0.9rem;
  }
  input {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text);
    padding: 10px 12px;
    font-size: 1rem;
    outline: none;
  }
  input:focus {
    border-color: var(--accent);
  }
  .err {
    margin: 0;
    color: var(--danger, #d9684a);
    font-size: 0.85rem;
  }
  .recovery {
    margin: 0;
    color: var(--text-muted);
    font-size: 0.78rem;
    line-height: 1.5;
    text-align: left;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 10px 12px;
  }
  .btn-primary {
    margin-top: 2px;
  }
</style>
