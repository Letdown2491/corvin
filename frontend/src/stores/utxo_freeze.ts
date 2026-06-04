import { writable } from 'svelte/store'
import { api } from '../lib/api'
import { swallow } from '../lib/utils'

export const frozenUtxos = writable<Set<string>>(new Set())

export async function loadFrozenUtxos() {
  try {
    frozenUtxos.set(new Set(await api.utxoFreeze.list()))
  } catch (e) { swallow(e, 'loadFrozenUtxos') }
}

export async function freezeUtxo(txid: string, vout: number) {
  await api.utxoFreeze.freeze(txid, vout)
  frozenUtxos.update(s => new Set([...s, `${txid}:${vout}`]))
}

export async function unfreezeUtxo(txid: string, vout: number) {
  await api.utxoFreeze.unfreeze(txid, vout)
  frozenUtxos.update(s => { const n = new Set(s); n.delete(`${txid}:${vout}`); return n })
}
