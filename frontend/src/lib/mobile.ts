import { browser } from '$app/environment'
import { readable } from 'svelte/store'

export const isMobile = readable(false, set => {
  if (!browser) return
  const mq = window.matchMedia('(max-width: 768px)')
  set(mq.matches)
  const handler = (e: MediaQueryListEvent) => set(e.matches)
  mq.addEventListener('change', handler)
  return () => mq.removeEventListener('change', handler)
})
