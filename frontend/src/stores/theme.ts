import { browser } from '$app/environment'
import { writable } from 'svelte/store'

/// 'auto' tracks the OS prefers-color-scheme media query.
export type ColorScheme = 'dark' | 'light' | 'auto'
type ResolvedScheme = 'dark' | 'light'

function resolveScheme(s: ColorScheme): ResolvedScheme {
  if (s === 'auto') {
    if (!browser) return 'dark'
    return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
  }
  return s
}

export interface AccentColor { name: string; value: string }

export const ACCENT_COLORS: AccentColor[] = [
  { name: 'Orange',  value: '#f7931a' },
  { name: 'Red',     value: '#e05252' },
  { name: 'Blue',    value: '#4f8ef7' },
  { name: 'Green',   value: '#52a875' },
  { name: 'Purple',  value: '#a855f7' },
  { name: 'Cyan',    value: '#06c4c4' },
]

const SCHEME_VARS: Record<ResolvedScheme, Record<string, string>> = {
  dark: {
    '--surface-1':     '#0d0d0d',
    '--surface-2':     '#181818',
    '--surface-hover': '#1c1c1c',
    '--border':        '#2a2a2a',
    '--text':          '#e8e8e8',
    '--text-muted':    '#c2c2c2',
    '--error':         '#e05252',
    'color-scheme':    'dark',
  },
  light: {
    '--surface-1':     '#f5f5f5',
    '--surface-2':     '#e9e9e9',
    '--surface-hover': '#e0e0e0',
    '--border':        '#cccccc',
    '--text':          '#111111',
    '--text-muted':    '#454545',
    '--error':         '#cc3333',
    'color-scheme':    'light',
  },
}

export interface Theme { scheme: ColorScheme; accent: string }

function load(): Theme {
  const stored = browser ? (localStorage.getItem('theme-scheme') as ColorScheme | null) : null
  const valid = stored === 'dark' || stored === 'light' || stored === 'auto' ? stored : 'dark'
  return {
    scheme: valid,
    accent: (browser ? localStorage.getItem('theme-accent') : null) ?? '#f7931a',
  }
}

function apply(t: Theme) {
  if (!browser) return
  const root = document.documentElement
  const resolved = resolveScheme(t.scheme)
  for (const [k, v] of Object.entries(SCHEME_VARS[resolved])) {
    root.style.setProperty(k, v)
  }
  root.style.setProperty('--accent', t.accent)
}

export const theme = writable<Theme>(load())

theme.subscribe(t => {
  if (!browser) return
  localStorage.setItem('theme-scheme', t.scheme)
  localStorage.setItem('theme-accent', t.accent)
  apply(t)
})

// React to OS dark-mode changes while the user has 'auto' set.
if (browser) {
  const mq = window.matchMedia('(prefers-color-scheme: light)')
  mq.addEventListener('change', () => {
    // Re-apply with the current store value — only effective when scheme === 'auto'.
    theme.update(t => t)
  })
}
