<script lang="ts">
  // Popover to assign a coin/address to a category: pick an existing one, clear
  // it, create a new one (with starter suggestions), or delete a definition.
  // Controlled value in via `current`; assignment out via `onSelect`. Category
  // definitions themselves live in the categories store (create/delete here).
  import { categoryDefs, createCategory, updateCategory, deleteCategory } from '../stores/categories'
  import type { Category } from '../lib/types'
  import CategoryChip from './CategoryChip.svelte'

  let {
    current,
    onSelect,
    inherited = false,
  }: {
    current: string | null
    onSelect: (id: string | null) => void
    /// True when `current` comes from the address's category (not a per-coin
    /// override) — shown with an "inherited" hint so picking is clearly an override.
    inherited?: boolean
  } = $props()

  const PALETTE = ['#e0623b', '#e0a93b', '#52a875', '#5b9bd5', '#a87fd4', '#d45b8e', '#52a8a8', '#9ca3af']
  const SUGGESTIONS = [
    { name: 'KYC', color: '#e0623b' },
    { name: 'Private', color: '#52a875' },
    { name: 'Savings', color: '#5b9bd5' },
    { name: 'Income', color: '#e0a93b' },
  ]

  let open = $state(false)
  let creating = $state(false)
  let editingId = $state<string | null>(null)   // null = creating; else editing this id
  let newName = $state('')
  let newColor = $state(PALETTE[0])
  let busy = $state(false)

  // The dropdown is fixed-positioned (anchored to the trigger) so a table's
  // overflow can't clip it — the bug where the menu "didn't open" was really it
  // being clipped inside the scrolling table body.
  let triggerEl = $state<HTMLButtonElement | null>(null)
  let menuPos = $state({ top: 0, left: 0 })

  function toggle(e: MouseEvent) {
    e.stopPropagation()
    if (!open && triggerEl) {
      const r = triggerEl.getBoundingClientRect()
      menuPos = { top: r.bottom + 4, left: r.left }
    }
    open = !open
    if (!open) { creating = false; editingId = null }
  }

  function pick(id: string | null) {
    onSelect(id)
    open = false
  }

  function startCreate() {
    editingId = null; newName = ''; newColor = PALETTE[0]; creating = true
  }
  function startEdit(c: Category) {
    editingId = c.id; newName = c.name; newColor = c.color; creating = true
  }

  // Save the inline form — create a new category (then assign it) or, when
  // editingId is set, rename/recolor the existing one.
  async function save() {
    if (!newName.trim() || busy) return
    busy = true
    try {
      if (editingId) {
        await updateCategory(editingId, newName.trim(), newColor)
        editingId = null; creating = false
      } else {
        const cat = await createCategory(newName.trim(), newColor)
        onSelect(cat.id)
        newName = ''; creating = false; open = false
      }
    } finally { busy = false }
  }

  async function quickAdd(name: string, color: string) {
    if (busy) return
    busy = true
    try {
      const cat = await createCategory(name, color)
      onSelect(cat.id)
      open = false
    } finally { busy = false }
  }

  async function remove(id: string) {
    if (busy) return
    busy = true
    try { await deleteCategory(id) } finally { busy = false }
  }
</script>

<div class="cat-picker">
  <button
    type="button"
    bind:this={triggerEl}
    class="cat-trigger"
    class:inherited={inherited && current}
    onclick={toggle}
    title={inherited && current ? 'Inherited from this address — pick to set a category just for this coin' : 'Set category'}
  >
    {#if current}
      <CategoryChip categoryId={current} />
      {#if inherited}<span class="cat-inherit-mark" aria-hidden="true">↳</span>{/if}
    {:else}
      <span class="cat-add">＋ category</span>
    {/if}
  </button>

  {#if open}
    <button class="cat-backdrop" aria-label="Close" tabindex="-1" onclick={() => { open = false; creating = false; editingId = null }}></button>
    <div class="cat-menu" role="menu" style="top: {menuPos.top}px; left: {menuPos.left}px">
      {#if $categoryDefs.length > 0}
        {#each $categoryDefs as c (c.id)}
          <div class="cat-row" class:active={current === c.id}>
            <button type="button" class="cat-row-pick" role="menuitemradio" aria-checked={current === c.id} onclick={() => pick(c.id)}>
              <CategoryChip categoryId={c.id} />
            </button>
            <button type="button" class="cat-row-edit" title="Rename / recolor" aria-label={`Edit ${c.name}`} onclick={() => startEdit(c)}>✎</button>
            <button type="button" class="cat-row-del" title="Delete category" aria-label={`Delete ${c.name}`} onclick={() => remove(c.id)}>✕</button>
          </div>
        {/each}
        <button type="button" class="cat-clear" role="menuitem" onclick={() => pick(null)}>No category</button>
        <div class="cat-divider"></div>
      {/if}

      {#if creating}
        <div class="cat-create">
          <input
            class="cat-name-input"
            placeholder="Category name"
            bind:value={newName}
            onkeydown={(e) => { if (e.key === 'Enter') save() }}
          />
          <div class="cat-swatches">
            {#each PALETTE as col (col)}
              <button type="button" class="cat-swatch" class:sel={newColor === col} style="background: {col}" aria-label={`Color ${col}`} onclick={() => newColor = col}></button>
            {/each}
          </div>
          <button type="button" class="cat-create-btn" disabled={!newName.trim() || busy} onclick={save}>
            {busy ? 'Saving…' : editingId ? 'Save changes' : 'Create & assign'}
          </button>
        </div>
      {:else}
        {#if $categoryDefs.length === 0}
          <div class="cat-suggest-label">Suggestions</div>
          <div class="cat-suggest">
            {#each SUGGESTIONS as s (s.name)}
              <button type="button" class="cat-suggest-btn" style="--cat: {s.color}" disabled={busy} onclick={() => quickAdd(s.name, s.color)}>
                <span class="cat-dot" style="background: {s.color}"></span>{s.name}
              </button>
            {/each}
          </div>
        {/if}
        <button type="button" class="cat-new" onclick={startCreate}>＋ New category</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .cat-picker { position: relative; display: inline-flex; }
  .cat-trigger { background: none; border: none; padding: 0; cursor: pointer; display: inline-flex; }
  .cat-add {
    font-size: 0.66rem; color: var(--text-muted);
    border: 1px dashed var(--border); border-radius: 999px; padding: 2px 7px;
  }
  .cat-trigger:hover .cat-add { color: var(--text); border-color: var(--text-muted); }

  .cat-backdrop { position: fixed; inset: 0; z-index: 19; background: transparent; border: none; cursor: default; padding: 0; }
  .cat-menu {
    position: fixed; z-index: 20;
    background: var(--surface-1); border: 1px solid var(--border); border-radius: 7px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.4); padding: 5px; min-width: 180px;
    display: flex; flex-direction: column; gap: 2px;
  }
  .cat-row { display: flex; align-items: center; gap: 4px; border-radius: 4px; }
  .cat-row:hover { background: var(--surface-hover); }
  .cat-row.active { background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .cat-row-pick { flex: 1; background: none; border: none; text-align: left; padding: 5px 6px; cursor: pointer; }
  .cat-row-edit { background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 0.72rem; padding: 4px 4px; }
  .cat-row-edit:hover { color: var(--accent); }
  .cat-row-del { background: none; border: none; color: var(--text-muted); cursor: pointer; font-size: 0.7rem; padding: 4px 7px; }
  .cat-row-del:hover { color: #e05252; }
  /* Inherited (from the address) reads lighter + a ↳ marker, so a per-coin
     override is visibly distinct from an inherited category. */
  .cat-trigger.inherited { opacity: 0.75; }
  .cat-inherit-mark { font-size: 0.66rem; color: var(--text-muted); margin-left: 2px; align-self: center; }
  .cat-clear, .cat-new {
    background: none; border: none; text-align: left; padding: 6px; cursor: pointer;
    font-size: 0.78rem; color: var(--text-muted); border-radius: 4px;
  }
  .cat-clear:hover, .cat-new:hover { background: var(--surface-hover); color: var(--text); }
  .cat-divider { height: 1px; background: var(--border); margin: 3px 0; }

  .cat-suggest-label { font-size: 0.64rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--text-muted); padding: 2px 6px; }
  .cat-suggest { display: flex; flex-wrap: wrap; gap: 4px; padding: 2px 4px 4px; }
  .cat-suggest-btn {
    display: inline-flex; align-items: center; gap: 4px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 999px;
    padding: 3px 8px; cursor: pointer; font-size: 0.7rem; color: var(--text);
  }
  .cat-suggest-btn:hover { border-color: var(--cat); }
  .cat-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }

  .cat-create { display: flex; flex-direction: column; gap: 6px; padding: 4px; }
  .cat-name-input {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 4px;
    color: var(--text); padding: 5px 8px; font-size: 0.78rem; outline: none; width: 100%; box-sizing: border-box;
  }
  .cat-name-input:focus { border-color: var(--accent); }
  .cat-swatches { display: flex; flex-wrap: wrap; gap: 5px; }
  .cat-swatch { width: 18px; height: 18px; border-radius: 50%; border: 2px solid transparent; cursor: pointer; padding: 0; }
  .cat-swatch.sel { border-color: var(--text); }
  .cat-create-btn {
    background: var(--accent); color: #000; border: none; border-radius: 4px;
    padding: 5px 10px; cursor: pointer; font-size: 0.76rem; font-weight: 600;
  }
  .cat-create-btn:disabled { opacity: 0.4; cursor: not-allowed; }
</style>
