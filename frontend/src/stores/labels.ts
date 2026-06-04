import { writable } from 'svelte/store'
import { api } from '../lib/api'
import { swallow } from '../lib/utils'

export const labels = writable<Record<string, string>>({})

export async function loadLabels() {
  try {
    labels.set(await api.labels.list())
  } catch (e) { swallow(e, 'loadLabels') }
}

export async function setLabel(txid: string, note: string) {
  await api.labels.set(txid, note)
  labels.update(l => {
    const next = { ...l }
    if (note.trim()) next[txid] = note.trim()
    else delete next[txid]
    return next
  })
}

