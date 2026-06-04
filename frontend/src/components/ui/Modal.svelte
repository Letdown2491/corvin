<script lang="ts">
  import type { Snippet } from 'svelte'

  // Shell over the native <dialog>: centralizes show/close, Esc (native cancel),
  // backdrop-click, focus trapping, and the header/✕/desc layout that 16 modals
  // were each re-implementing (and only 6 had Esc). Bind `open`; provide a body
  // (children) and an optional `footer` snippet for actions.
  let {
    open = $bindable(false),
    title,
    desc,
    width = '520px',
    onclose,
    children,
    footer,
    help,
  }: {
    open?: boolean
    title?: string
    desc?: string
    width?: string
    onclose?: () => void
    children: Snippet
    footer?: Snippet
    /// Optional accessory rendered next to the title (e.g. a HelpLink).
    help?: Snippet
  } = $props()

  let dialog = $state<HTMLDialogElement>()
  let panel = $state<HTMLDivElement>()
  // Unique ids so the dialog's accessible name/description point at the title/desc.
  const uid = `modal-${Math.random().toString(36).slice(2, 9)}`
  // Where to send focus back when the modal closes (the element that opened it).
  let returnFocus: HTMLElement | null = null

  $effect(() => {
    if (!dialog) return
    if (open && !dialog.open) {
      returnFocus = (document.activeElement as HTMLElement) ?? null
      dialog.showModal()
      // Land focus on the panel/heading, not the first tabbable (usually the ✕).
      panel?.focus()
    } else if (!open && dialog.open) {
      dialog.close()
    }
  })

  // Fires on Esc (native cancel→close) and on programmatic close — keep `open` in
  // sync, notify the parent once, and return focus to the trigger.
  function handleClose() {
    if (open) {
      open = false
      onclose?.()
      returnFocus?.focus?.()
      returnFocus = null
    }
  }
</script>

<dialog
  bind:this={dialog}
  class="ui-modal"
  aria-modal="true"
  aria-labelledby={title ? `${uid}-title` : undefined}
  aria-describedby={desc ? `${uid}-desc` : undefined}
  onclose={handleClose}
  onclick={(e) => { if (e.target === dialog) handleClose() }}
>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="panel" style={`width: ${width}`} bind:this={panel} tabindex="-1">
    <div class="modal-header">
      {#if title}<h2 class="modal-title" id={`${uid}-title`}>{title}{#if help} {@render help()}{/if}</h2>{/if}
      <button type="button" class="close-btn" onclick={handleClose} aria-label="Close">✕</button>
    </div>
    {#if desc}<p class="modal-desc" id={`${uid}-desc`}>{desc}</p>{/if}
    <div class="modal-body">{@render children()}</div>
    {#if footer}<div class="modal-footer">{@render footer()}</div>{/if}
  </div>
</dialog>

<style>
  /* Canonical modal look for the whole app: centered card with a drop shadow over
     a blurred backdrop on desktop; a bottom-sheet on mobile. */
  .ui-modal {
    position: fixed; inset: 0; margin: auto;
    border: none; background: transparent; padding: 0; color: inherit;
    max-height: min(92vh, 720px);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    border-radius: 10px;
  }
  .ui-modal::backdrop { background: rgba(0, 0, 0, 0.6); backdrop-filter: blur(2px); }
  .panel {
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 10px; max-width: calc(100vw - 32px); max-height: inherit;
    box-sizing: border-box; overflow-y: auto;
    padding: 18px 22px 22px; display: flex; flex-direction: column; gap: 14px;
  }
  .modal-header { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
  .modal-title { font-size: 1rem; font-weight: 700; color: var(--text); margin: 0; }
  .modal-desc { font-size: 0.8rem; color: var(--text-muted); margin: 0; line-height: 1.5; }
  .modal-body { display: flex; flex-direction: column; gap: 12px; }
  .modal-footer { display: flex; justify-content: flex-end; gap: 8px; }

  @media (max-width: 768px) {
    .ui-modal {
      margin: auto 0 0; border-radius: 12px 12px 0 0; border-bottom: none;
      max-height: 94vh;
    }
    .panel { width: 100vw !important; max-width: 100vw; border-radius: 12px 12px 0 0; }
  }
</style>
