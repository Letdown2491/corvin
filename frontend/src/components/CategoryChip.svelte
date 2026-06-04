<script lang="ts">
  // Small colored chip for a coin category, looked up by id. Renders nothing when
  // the coin has no category (so callers can drop it in unconditionally).
  import { categoryById } from '../stores/categories'

  let { categoryId, dotOnly = false }: { categoryId: string | null; dotOnly?: boolean } = $props()

  let cat = $derived(categoryId ? $categoryById[categoryId] ?? null : null)
</script>

{#if cat}
  <span class="cat-chip" class:dot-only={dotOnly} title={cat.name} style="--cat: {cat.color}">
    <span class="cat-dot"></span>
    {#if !dotOnly}<span class="cat-name">{cat.name}</span>{/if}
  </span>
{/if}

<style>
  .cat-chip {
    display: inline-flex; align-items: center; gap: 4px;
    font-size: 0.66rem; line-height: 1;
    color: var(--cat);
    background: color-mix(in srgb, var(--cat) 14%, transparent);
    border: 1px solid color-mix(in srgb, var(--cat) 35%, transparent);
    border-radius: 999px; padding: 2px 7px 2px 5px;
    white-space: nowrap; max-width: 140px;
  }
  .cat-chip.dot-only { padding: 0; background: none; border: none; }
  .cat-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--cat); flex-shrink: 0; }
  .cat-name { overflow: hidden; text-overflow: ellipsis; }
</style>
