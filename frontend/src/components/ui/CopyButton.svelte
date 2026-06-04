<script lang="ts">
  import { addToast } from '../../stores/toasts'

  // Copy text to the clipboard with a transient "Copied ✓" flash + a failure
  // toast. `text` is a string or a getter (so the value can be computed lazily).
  let {
    text,
    idle = 'Copy',
    copiedLabel = 'Copied ✓',
    variant = 'secondary',
    class: klass = '',
    title = '',
    disabled = false,
  }: {
    text: string | (() => string)
    idle?: string
    copiedLabel?: string
    variant?: 'primary' | 'secondary' | 'danger' | 'ghost'
    /// Override the default `btn-{variant}` class to match a bespoke button style.
    class?: string
    title?: string
    disabled?: boolean
  } = $props()

  let copied = $state(false)
  let timer: ReturnType<typeof setTimeout> | null = null

  async function copy() {
    try {
      await navigator.clipboard.writeText(typeof text === 'function' ? text() : text)
      copied = true
      if (timer) clearTimeout(timer)
      timer = setTimeout(() => (copied = false), 1500)
    } catch {
      addToast('Copy failed — clipboard unavailable')
    }
  }

  $effect(() => () => { if (timer) clearTimeout(timer) })
</script>

<button type="button" class={klass || `btn-${variant}`} {title} {disabled} onclick={copy}>
  {copied ? copiedLabel : idle}
</button>
