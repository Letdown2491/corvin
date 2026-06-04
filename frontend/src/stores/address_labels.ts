import { writable } from 'svelte/store'
import { api } from '../lib/api'
import { swallow } from '../lib/utils'

// Shared so an address note set on the Addresses tab is reactively reflected
// everywhere it's inherited (UTXO notes fall back to the address note, coin
// control, receive, send). Previously this was per-component state passed as a
// prop, so a save didn't propagate to siblings until a full reload.
export const addressLabels = writable<Record<string, string>>({})

export async function loadAddressLabels() {
  try {
    addressLabels.set(await api.addressLabels.list())
  } catch (e) { swallow(e, 'loadAddressLabels') }
}

export async function setAddressLabel(address: string, note: string) {
  const trimmed = note.trim()
  if (trimmed) await api.addressLabels.set(address, trimmed)
  else await api.addressLabels.delete(address)
  addressLabels.update(m => {
    const next = { ...m }
    if (trimmed) next[address] = trimmed
    else delete next[address]
    return next
  })
}
