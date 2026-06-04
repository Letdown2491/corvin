import { writable } from 'svelte/store'
import type { PayjoinStatusKind } from '../lib/types'

/// Last payjoin SSE event, so an open SendModal can react to a proposal
/// arriving (or the session falling back) without polling. Null until the
/// first event of a session.
export interface PayjoinEvent {
  wallet_id?: string
  session_id: string
  status: Extract<PayjoinStatusKind, 'proposal_ready' | 'sent' | 'fell_back'>
  txid?: string
}

export const lastPayjoinEvent = writable<PayjoinEvent | null>(null)
