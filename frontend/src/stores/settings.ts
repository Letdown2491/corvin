import { browser } from '$app/environment'
import { writable, derived } from 'svelte/store'
import type { BackendStatusEntry, FeeRates, NodeStatus } from '../lib/types'
export type { FeeRates } from '../lib/types'

export const nodeStatus = writable<NodeStatus | null>(null)
/// Timestamp of the last successful refresh — sidebar uses this for the stale indicator.
export const nodeStatusAt = writable<number>(0)

/// Whether native-USB hardware-wallet support is compiled into the backend
/// (from /version `hw_enabled`). False on the Start9/headless build, where the
/// UI hides USB-connect/sign affordances and signs via QR/PSBT instead. Default
/// true so the desktop build (and any failed version probe) keeps HW visible.
export const hwEnabled = writable<boolean>(true)

/// Air-gapped/offline mode (from /status). When true, the instance never opens a
/// backend connection: reads serve cached state, sync + broadcast are unavailable,
/// and signing/PSBT export still work.
export const offline = derived(nodeStatus, ($n) => $n?.offline ?? false)

/// Live per-backend connection state (keyed by backend id; null = default).
/// Drives the per-wallet connection dot in the sidebar and the wallet page.
export const backendStatuses = writable<BackendStatusEntry[]>([])
export const backendStatusesAt = writable<number>(0)

export const feeRates = writable<FeeRates | null>(null)
export const feeRatesAt = writable<number>(0)

const _storedUnit = browser ? localStorage.getItem('displayUnit') : null
export const displayUnit = writable<'btc' | 'sats'>(
  _storedUnit === 'btc' || _storedUnit === 'sats' ? _storedUnit : 'sats'
)
displayUnit.subscribe(v => { if (browser) localStorage.setItem('displayUnit', v) })

export const balancesHidden = writable<boolean>(
  browser ? localStorage.getItem('balancesHidden') === 'true' : false
)
balancesHidden.subscribe(v => { if (browser) localStorage.setItem('balancesHidden', String(v)) })

export const mempoolUrl = writable<string>('https://mempool.space')

export const showPriceData = writable<boolean>(false)
export const showCurrentPrice = writable<boolean>(false)
export const showFiatBalance = writable<boolean>(false)
export const currentBtcPrice = writable<number | null>(null)

export const defaultWalletId = writable<string | null>(
  browser ? (localStorage.getItem('defaultWalletId') ?? null) : null
)
defaultWalletId.subscribe(v => {
  if (!browser) return
  if (v) localStorage.setItem('defaultWalletId', v)
  else localStorage.removeItem('defaultWalletId')
})

export const notificationsEnabled = writable<boolean>(
  browser ? localStorage.getItem('notificationsEnabled') === 'true' : false
)
notificationsEnabled.subscribe(v => { if (browser) localStorage.setItem('notificationsEnabled', String(v)) })
