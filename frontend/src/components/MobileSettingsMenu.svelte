<script lang="ts">
  import { goto } from '$app/navigation'
  import { slideDir } from '../stores/ui'

  const items = [
    { label: 'Backend',         icon: '◎', href: '/settings/backend' },
    { label: 'Display',         icon: '◑', href: '/settings/display' },
    { label: 'Security',        icon: '🔒', href: '/settings/security' },
    { label: 'Import / Export', icon: '⇅', href: '/settings/import-export' },
    { label: 'Help',            icon: '?', href: '/help' },
  ]

  function navigate(href: string) {
    slideDir.set(1)
    goto(href)
  }
</script>

<div class="settings-menu">
  {#each items as item (item.href)}
    <button class="menu-row" onclick={() => navigate(item.href)}>
      <span class="menu-icon" aria-hidden="true">{item.icon}</span>
      <span class="menu-label">{item.label}</span>
      <span class="chevron" aria-hidden="true">›</span>
    </button>
  {/each}
  <a class="menu-row" href="/bitcoin.pdf" target="_blank" rel="noopener noreferrer">
    <span class="menu-icon" aria-hidden="true">₿</span>
    <span class="menu-label">Whitepaper</span>
    <span class="chevron" aria-hidden="true">↗</span>
  </a>
</div>

<style>
  .settings-menu { display: flex; flex-direction: column; }

  .menu-row {
    display: flex; align-items: center; gap: 16px;
    padding: 18px 20px; background: none; border: none;
    border-bottom: 1px solid var(--border); cursor: pointer;
    color: var(--text); text-align: left; width: 100%;
    min-height: 60px;
  }
  .menu-row:active { background: var(--surface-hover); }
  a.menu-row { text-decoration: none; }

  .menu-icon { font-size: 1rem; width: 22px; text-align: center; flex-shrink: 0; color: var(--text-muted); font-variant-emoji: text; }
  .menu-label { font-size: 1rem; font-weight: 500; flex: 1; }
  .chevron { font-size: 1.6rem; color: var(--text-muted); line-height: 1; }
</style>
