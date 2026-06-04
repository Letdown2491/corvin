<script lang="ts">
  import type { Snippet } from 'svelte'

  // A consistent zero-state: a small monochrome line icon, a title, an optional
  // description, and an optional action (passed as a snippet). Keeps every empty
  // state in the app visually + structurally uniform instead of one-off <p>s.
  let {
    icon = 'inbox',
    title,
    description = '',
    action,
    compact = false,
  }: {
    /// Which built-in line icon to show. Add cases below as needed.
    icon?: 'inbox' | 'sync' | 'search' | 'plug' | 'coins' | 'wallet'
    title: string
    description?: string
    /// Optional action area (e.g. a button), rendered under the text.
    action?: Snippet
    /// Tighter padding for use inside a tab/table rather than a full page.
    compact?: boolean
  } = $props()
</script>

<div class="empty-state" class:compact role="status">
  <span class="es-icon" aria-hidden="true">
    {#if icon === 'sync'}
      <svg class="es-spin" viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round">
        <path d="M20 12a8 8 0 1 1-2.3-5.6" /><path d="M20 4v4h-4" />
      </svg>
    {:else if icon === 'search'}
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="11" cy="11" r="7" /><path d="M21 21l-4.3-4.3" />
      </svg>
    {:else if icon === 'plug'}
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M9 2v6M15 2v6" /><path d="M6 8h12v3a6 6 0 0 1-12 0z" /><path d="M12 17v5" />
      </svg>
    {:else if icon === 'coins'}
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <ellipse cx="12" cy="6" rx="8" ry="3" /><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6" /><path d="M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" />
      </svg>
    {:else if icon === 'wallet'}
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 7a2 2 0 0 1 2-2h12a2 2 0 0 1 2 2v0H5a2 2 0 0 0-2 2z" /><rect x="3" y="7" width="18" height="12" rx="2" /><circle cx="16.5" cy="13" r="1.2" />
      </svg>
    {:else}
      <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 13l3-8h12l3 8" /><path d="M3 13h5l1 3h6l1-3h5v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
      </svg>
    {/if}
  </span>
  <p class="es-title">{title}</p>
  {#if description}<p class="es-desc">{description}</p>{/if}
  {#if action}<div class="es-action">{@render action()}</div>{/if}
</div>

<style>
  .empty-state {
    display: flex; flex-direction: column; align-items: center; text-align: center;
    gap: 8px; padding: 48px 24px; color: var(--text-muted);
  }
  .empty-state.compact { padding: 28px 16px; }
  .es-icon { color: var(--text-muted); opacity: 0.7; margin-bottom: 2px; line-height: 0; }
  .es-spin { animation: es-spin 1s linear infinite; transform-origin: center; }
  @keyframes es-spin { to { transform: rotate(360deg); } }
  .es-title { font-size: 0.95rem; font-weight: 600; color: var(--text); margin: 0; }
  .es-desc { font-size: 0.85rem; color: var(--text-muted); margin: 0; max-width: 340px; line-height: 1.5; }
  .es-action { margin-top: 10px; }
</style>
