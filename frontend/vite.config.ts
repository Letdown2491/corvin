import { defineConfig } from 'vite'
import { sveltekit } from '@sveltejs/kit/vite'

export default defineConfig({
  plugins: [sveltekit()],
  build: {
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        // QR / signing libraries are most of the bundle weight and only run
        // inside Send / Receive / QR-sign flows. Two groups, because they reach
        // the bundle differently:
        //   • qr-decode = jsqr (~140 kB) + qrcode — imported *dynamically only*
        //     (jsqr at scan start, qrcode at display time), so this chunk is
        //     async and stays out of the eager wallet route entirely.
        //   • qr-libs = bbqr + @ngraveio/bc-ur + buffer — statically imported by
        //     lib/qr.ts's synchronous frame collector, so they're eager; keeping
        //     them separate stops them from dragging jsqr/qrcode along.
        // (Function form so we skip the SSR pass where these are external.)
        manualChunks(id) {
          if (!id.includes('node_modules')) return
          if (id.includes('/jsqr/') || id.includes('/qrcode/')) {
            return 'qr-decode'
          }
          if (
            id.includes('/bbqr/') ||
            id.includes('/@ngraveio/') ||
            id.includes('/buffer/')
          ) {
            return 'qr-libs'
          }
        },
      },
    },
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:5757',
        // Disable response buffering so SSE events are forwarded to the browser immediately.
        configure: (proxy) => {
          proxy.on('proxyReq', (proxyReq, req) => {
            if (req.headers.accept?.includes('text/event-stream')) {
              proxyReq.setHeader('Connection', 'keep-alive')
            }
          })
        },
      },
    },
  },
})
