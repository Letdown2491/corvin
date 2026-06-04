import { writable } from 'svelte/store'
import { api } from '../lib/api'
import { swallow } from '../lib/utils'

export const utxoLabels = writable<Record<string, string>>({})

export async function loadUtxoLabels() {
  try {
    utxoLabels.set(await api.utxoLabels.list())
  } catch (e) { swallow(e, 'loadUtxoLabels') }
}

export async function setUtxoLabel(txid: string, vout: number, note: string) {
  await api.utxoLabels.set(txid, vout, note)
  const key = `${txid}:${vout}`
  utxoLabels.update(m => {
    if (!note.trim()) {
      const next = { ...m }
      delete next[key]
      return next
    }
    return { ...m, [key]: note.trim() }
  })
}
