<script lang="ts">
  import type { Snippet } from 'svelte'

  // Owns the busy flag around an async onclick so call sites stop hand-rolling
  // `busy = $state(false)` + try/finally + a disabled + label ternary. The
  // handler does its own error handling; we just manage busy/disabled/label.
  let {
    onclick,
    idle,
    busyLabel,
    variant = 'primary',
    class: klass = '',
    type = 'button',
    disabled = false,
    title = '',
    busy = $bindable(false),
    children,
  }: {
    onclick?: (e: MouseEvent) => unknown | Promise<unknown>
    idle?: string
    busyLabel?: string
    variant?: 'primary' | 'secondary' | 'danger' | 'ghost'
    /// Override the default `btn-{variant}` class to match a bespoke button style.
    class?: string
    type?: 'button' | 'submit'
    disabled?: boolean
    title?: string
    busy?: boolean
    children?: Snippet
  } = $props()

  async function handle(e: MouseEvent) {
    if (busy) return
    busy = true
    try {
      await onclick?.(e)
    } finally {
      busy = false
    }
  }
</script>

<button
  {type}
  class={klass || `btn-${variant}`}
  disabled={busy || disabled}
  {title}
  onclick={handle}
>
  {#if children}{@render children()}{:else}{busy ? busyLabel : idle}{/if}
</button>
