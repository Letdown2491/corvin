import { writable } from 'svelte/store'

/** True when the Corvin server last responded; false when a network-level fetch failed. */
export const serverReachable = writable(true)
