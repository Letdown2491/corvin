/// Curated public Electrum servers offered as backend choices (global default
/// and per-wallet). Not stored in the saved-backends registry — selecting one
/// either writes the connection into the global default, or (per wallet)
/// materializes a matching saved backend to pin to.
export const PUBLIC_SERVERS = [
  { host: 'electrum.blockstream.info',  port: 50002, ssl: true },
  { host: 'electrum.diynodes.com',      port: 50002, ssl: true },
  { host: 'fulcrum.sethforprivacy.com', port: 50002, ssl: true },
] as const

export type PublicServer = typeof PUBLIC_SERVERS[number]
