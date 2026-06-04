import { writable } from 'svelte/store'
import { api } from '../lib/api'
import { swallow } from '../lib/utils'

export const costBasis = writable<Record<string, number>>({})

export async function loadCostBasis() {
  try {
    costBasis.set(await api.costBasis.list())
  } catch (e) { swallow(e, 'loadCostBasis') }
}

export async function setCostBasis(txid: string, usd: number) {
  await api.costBasis.set(txid, usd)
  costBasis.update(cb => ({ ...cb, [txid]: usd }))
}

export async function deleteCostBasis(txid: string) {
  await api.costBasis.delete(txid)
  costBasis.update(cb => { const next = { ...cb }; delete next[txid]; return next })
}
