<script lang="ts">
  import { goto } from '$app/navigation'
  import { untrack } from 'svelte'
  import MobileSettingsMenu from '../../components/MobileSettingsMenu.svelte'
  import { isMobile } from '../../lib/mobile'

  $effect(() => {
    if (!$isMobile) untrack(() => goto('/settings/backend', { replaceState: true }))
  })
</script>

{#if $isMobile}
  <MobileSettingsMenu />
{/if}
