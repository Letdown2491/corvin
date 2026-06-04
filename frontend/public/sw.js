// Bump on any change to this file's strategy so `activate` evicts old caches.
const CACHE = 'corvin-v3'

self.addEventListener('install', () => self.skipWaiting())
self.addEventListener('activate', (e) => {
  e.waitUntil(
    caches.keys()
      .then(keys => Promise.all(keys.filter(k => k !== CACHE).map(k => caches.delete(k))))
      .then(() => self.clients.claim())
  )
})

self.addEventListener('fetch', (e) => {
  // Pass API and SSE requests straight through — never cache them.
  const url = new URL(e.request.url)
  if (url.pathname.startsWith('/api/')) return

  // Network-first. The backend is in-process (desktop) or local (Start9), so it's
  // effectively always available — prefer it and only fall back to cache when
  // genuinely offline. A cache-first shell served a stale index.html after each
  // rebuild/update, which then 404'd the new hashed chunks and blanked the app.
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        if (res.ok) {
          const clone = res.clone()
          caches.open(CACHE).then((c) => c.put(e.request, clone))
        }
        return res
      })
      .catch(() =>
        caches.match(e.request).then((cached) => {
          if (cached) return cached
          // Offline SPA deep-link reload (e.g. /wallet/123): no per-route entry is
          // cached, so fall back to the cached app shell and let the client router
          // take over. Only on a genuine network failure — fresh index still wins
          // when online, so no stale-shell regression.
          if (e.request.mode === 'navigate') {
            return caches.match('/').then((shell) => shell ?? Promise.reject(new Error('offline')))
          }
          return Promise.reject(new Error('offline'))
        }),
      ),
  )
})
