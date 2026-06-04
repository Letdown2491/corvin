<script lang="ts">
  import { toasts } from '../stores/toasts'
</script>

<div class="toast-stack" role="status" aria-live="polite" aria-atomic="true">
  {#each $toasts as toast (toast.id)}
    <div class="toast">{toast.message}</div>
  {/each}
</div>

<style>
  .toast-stack {
    position: fixed;
    bottom: 24px;
    right: 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 1000;
  }
  .toast {
    background: var(--surface-1);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 10px 16px;
    border-radius: 6px;
    font-size: 0.85rem;
    box-shadow: 0 4px 16px rgba(0,0,0,0.4);
    border-left: 3px solid var(--accent);
    animation: slide-in 0.18s ease;
    max-width: 320px;
  }
  @media (max-width: 768px) {
    .toast-stack {
      bottom: calc(60px + env(safe-area-inset-bottom) + 12px);
      right: 12px;
      left: 12px;
    }
    .toast { max-width: 100%; }
  }
  @keyframes slide-in {
    from { opacity: 0; transform: translateY(8px); }
    to   { opacity: 1; transform: translateY(0); }
  }
</style>
