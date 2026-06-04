import { writable, derived, get } from 'svelte/store'
import { api } from '../lib/api'
import { swallow, utxoKey } from '../lib/utils'
import type { Category, UtxoRecord } from '../lib/types'

// Category definitions and the two assignment maps. One store mirrors the
// backend's single categories.json so defs + assignments stay consistent.
export const categoryDefs = writable<Category[]>([])
export const addressCategories = writable<Record<string, string>>({})   // address → id
export const utxoCategories = writable<Record<string, string>>({})      // outpoint → id

// id → Category lookup for rendering chips.
export const categoryById = derived(categoryDefs, ($defs) => {
  const m: Record<string, Category> = {}
  for (const c of $defs) m[c.id] = c
  return m
})

export async function loadCategories() {
  try {
    const data = await api.categories.list()
    categoryDefs.set(data.definitions ?? [])
    addressCategories.set(data.addresses ?? {})
    utxoCategories.set(data.utxos ?? {})
  } catch (e) { swallow(e, 'loadCategories') }
}

// Effective category id of a coin: its own override, else its receiving
// address's category. Pure helper over the current store values.
export function effectiveCategoryId(utxo: UtxoRecord): string | null {
  const op = utxoKey(utxo.txid, utxo.vout)
  const own = get(utxoCategories)[op]
  if (own) return own
  if (utxo.address) {
    const addr = get(addressCategories)[utxo.address]
    if (addr) return addr
  }
  return null
}

export async function createCategory(name: string, color: string): Promise<Category> {
  const cat = await api.categories.create(name, color)
  categoryDefs.update(d => [...d, cat])
  return cat
}

export async function updateCategory(id: string, name: string, color: string) {
  await api.categories.update(id, name, color)
  categoryDefs.update(d => d.map(c => (c.id === id ? { ...c, name, color } : c)))
}

export async function deleteCategory(id: string) {
  await api.categories.delete(id)
  categoryDefs.update(d => d.filter(c => c.id !== id))
  // The backend drops assignments to a deleted category; mirror that locally.
  addressCategories.update(m => Object.fromEntries(Object.entries(m).filter(([, v]) => v !== id)))
  utxoCategories.update(m => Object.fromEntries(Object.entries(m).filter(([, v]) => v !== id)))
}

export async function assignAddressCategory(address: string, categoryId: string | null) {
  await api.categories.assignAddress(address, categoryId)
  addressCategories.update(m => {
    const next = { ...m }
    if (categoryId) next[address] = categoryId
    else delete next[address]
    return next
  })
}

export async function assignUtxoCategory(txid: string, vout: number, categoryId: string | null) {
  const op = utxoKey(txid, vout)
  await api.categories.assignUtxo(op, categoryId)
  utxoCategories.update(m => {
    const next = { ...m }
    if (categoryId) next[op] = categoryId
    else delete next[op]
    return next
  })
}
