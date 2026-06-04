import { writable } from 'svelte/store'
import type { SecurityState } from '../lib/types'

/** Current at-rest encryption state. 'unknown' until the first status fetch. */
export const securityState = writable<SecurityState | 'unknown'>('unknown')
