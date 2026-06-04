import { writable } from 'svelte/store'

export const slideDir = writable<-1 | 0 | 1>(1)
