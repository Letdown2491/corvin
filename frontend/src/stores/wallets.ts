import { writable } from 'svelte/store'
import type { Balance, WalletEntry } from '../lib/types'

export const wallets = writable<WalletEntry[]>([])
export const activeWalletId = writable<string | null>(null)
export const syncing = writable<Set<string>>(new Set())

/** Set on every sync_complete SSE event. `at` ensures re-trigger even for the same wallet. */
export const lastSyncComplete = writable<{ id: string; at: number } | null>(null)

/** Cached balance per wallet id, updated whenever WalletDetail loads balance. */
export const walletBalances = writable<Map<string, Balance>>(new Map())

/**
 * Wallet ids synced at least once this session. A wallet is auto-synced the
 * first time it's opened (so data is fresh after a restart), then left to the
 * subscriber's push/periodic sync — no redundant round-trip when bouncing
 * between wallets or returning from settings. Plain module state: it resets on
 * page reload, which counts as a restart. A wallet is marked either when we fire
 * its open-sync or when any `sync_complete` arrives for it (e.g. the subscriber's
 * startup sync), so we never double-sync.
 */
const syncedThisSession = new Set<string>()
export const markWalletSynced = (id: string) => {
  syncedThisSession.add(id)
}
/** Undo an optimistic mark when an open-sync fails, so a re-open retries. */
export const unmarkWalletSynced = (id: string) => {
  syncedThisSession.delete(id)
}
export const hasWalletSyncedThisSession = (id: string) => syncedThisSession.has(id)
