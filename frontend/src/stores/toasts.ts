import { writable } from 'svelte/store'

export interface Toast {
  id: number
  message: string
}

let nextId = 0
export const toasts = writable<Toast[]>([])

export function addToast(message: string) {
  const id = nextId++
  toasts.update(t => [...t, { id, message }])
  setTimeout(() => toasts.update(t => t.filter(x => x.id !== id)), 5000)
}
