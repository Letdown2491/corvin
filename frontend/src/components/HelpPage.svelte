<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import HelpContent from './HelpContent.svelte'

  // The set of categories, in rail order. Typing `cat` against this makes a
  // mistyped category a compile error rather than a silent phantom group.
  const CATEGORIES = [
    'Get started', 'Receiving', 'Sending', 'Wallets & devices',
    'Privacy', 'Backups & records', 'Tools', 'Troubleshooting', 'Concepts',
  ] as const
  type Category = (typeof CATEGORIES)[number]

  type Topic = { id: string; cat: Category; title: string; keywords: string }

  // Order here is the order in the rail. `cat` groups them. How-to guides lead
  // each task category; the explainer articles live under Concepts. The article
  // prose itself lives in HelpContent.svelte, keyed by these same ids — a dev
  // integrity check below asserts the two stay in sync.
  const topics: Topic[] = [
    { id: 'welcome',         cat: 'Get started',        title: 'What is Corvin?',           keywords: 'intro self-hosted node start welcome' },
    { id: 'choosing-setup',  cat: 'Get started',        title: 'Choosing your setup',       keywords: 'best practices security setup which wallet privacy savings spending hardware multisig node level' },
    { id: 'add-wallet',      cat: 'Get started',        title: 'Add your first wallet',     keywords: 'create new wallet single multisig watch only seed import xpub hardware' },
    { id: 'connect-backend', cat: 'Get started',        title: 'Connect a backend',         keywords: 'electrum bitcoin core knots rpc server node network mainnet testnet connect public private' },
    { id: 'offline',         cat: 'Get started',        title: 'Use Corvin offline',        keywords: 'offline air-gapped airgap air gap cold storage signer no backend disconnected sign export psbt' },

    { id: 'receive',         cat: 'Receiving',          title: 'Receive a payment',         keywords: 'receive address qr request amount label invoice get paid' },
    { id: 'sp-setup',        cat: 'Receiving',          title: 'Set up silent payments',    keywords: 'silent payments sp create scan spend watch only birthday' },

    { id: 'send',            cat: 'Sending',            title: 'Send a payment',            keywords: 'send pay recipient amount fee sign psbt broadcast preview review flow diagram two step' },
    { id: 'speed-up',        cat: 'Sending',            title: 'Speed up a stuck payment',  keywords: 'rbf cpfp replace fee bump stuck unconfirmed child parent' },
    { id: 'coin-control',    cat: 'Sending',            title: 'Use coin control',          keywords: 'coin control utxo freeze label note category select inputs choose coins' },

    { id: 'multisig-setup',  cat: 'Wallets & devices',  title: 'Set up a multisig wallet',  keywords: 'multisig setup signer cosigner threshold xpub descriptor m of n' },
    { id: 'hardware',        cat: 'Wallets & devices',  title: 'Sign with a hardware wallet', keywords: 'hardware bitbox ledger trezor coldcard sign device qr air-gapped' },

    { id: 'spending-privately', cat: 'Privacy',         title: 'Spending privately',        keywords: 'privacy overview coin control labels silent payments payjoin address reuse traceable' },
    { id: 'payjoin-send',    cat: 'Privacy',            title: 'Send a payjoin',            keywords: 'payjoin send invoice pj uri collaborative private fallback' },
    { id: 'payjoin-receive', cat: 'Privacy',            title: 'Receive a payjoin',         keywords: 'payjoin receive provision invoice node rpc contribute input' },
    { id: 'warnings',        cat: 'Privacy',            title: 'Privacy warnings',          keywords: 'warning address reuse round amount mixed labels categories reveal look-alike lookalike poisoning scam similar' },
    { id: 'encryption',      cat: 'Privacy',            title: 'Encrypt your data at rest', keywords: 'encryption encrypt password lock unlock at rest disk privacy config folder stolen copied seized backup vault argon2 security protect storage decrypt boot locked' },

    { id: 'backup',          cat: 'Backups & records',  title: 'Back up and restore',       keywords: 'backup restore bip329 labels export import recover' },
    { id: 'labels',          cat: 'Backups & records',  title: 'Notes and categories',      keywords: 'label note transaction address coin utxo annotate tag category categories colour color compartment chip' },
    { id: 'tax',             cat: 'Backups & records',  title: 'Export a tax report',       keywords: 'tax report gain loss fifo lifo hifo cost basis csv year' },
    { id: 'import-export',   cat: 'Backups & records',  title: 'Import and export data',    keywords: 'import export backup bip329 labels restore download passphrase' },

    { id: 'broadcast',       cat: 'Tools',              title: 'Broadcast a transaction',   keywords: 'broadcast psbt raw hex signed push send external txid' },
    { id: 'psbt-inspector',  cat: 'Tools',              title: 'Inspect a PSBT',            keywords: 'psbt inspect decode review fee inputs outputs without signing' },
    { id: 'consolidate',     cat: 'Tools',              title: 'Consolidate UTXOs',         keywords: 'consolidate utxo coins combine merge inputs cheaper fee dust' },
    { id: 'messages',        cat: 'Tools',              title: 'Sign or verify a message',  keywords: 'message sign verify bip322 prove ownership address signature' },
    { id: 'address-lookup',  cat: 'Tools',              title: 'Look up an address',        keywords: 'address lookup derivation belongs path script type check' },
    { id: 'export-keys',     cat: 'Tools',              title: 'Export keys and descriptors', keywords: 'export descriptor watch-only keys multisig config coordinator' },
    { id: 'sweep',           cat: 'Tools',              title: 'Sweep a private key',       keywords: 'sweep wif private key paper wallet drain import scan' },

    { id: 'troubleshooting', cat: 'Troubleshooting',    title: 'Troubleshooting',           keywords: 'problem error cant connect backend hardware wallet not detected passphrase missing funds disappeared certificate tls data folder fiat price' },

    { id: 'wallet-kinds',    cat: 'Concepts',           title: 'Wallet kinds explained',    keywords: 'single signature multisig silent payments watch-only vault policy timelock' },
    { id: 'addresses',       cat: 'Concepts',           title: 'Addresses',                 keywords: 'address reuse new gap limit privacy' },
    { id: 'seed',            cat: 'Concepts',           title: 'Recovery phrases',          keywords: 'seed mnemonic 12 24 words passphrase backup' },
    { id: 'sp-concept',      cat: 'Concepts',           title: 'How silent payments work',  keywords: 'silent payment bip352 reusable scan key server frigate trust' },
    { id: 'payjoin',         cat: 'Concepts',           title: 'How payjoin works',         keywords: 'payjoin bip77 collaborative private send receive' },
    { id: 'sending-concept', cat: 'Concepts',           title: 'How sending works',         keywords: 'psbt change broadcast unsigned sign transaction' },
    { id: 'fees',            cat: 'Concepts',           title: 'Fees and sat/vB',           keywords: 'fee rate satoshi virtual byte mempool confirmation' },
    { id: 'coins-concept',   cat: 'Concepts',           title: 'Coins, UTXOs, and change',  keywords: 'utxo coin change balance unspent output' },

    { id: 'glossary',        cat: 'Concepts',           title: 'Glossary',                  keywords: 'terms definitions words meaning' },
  ]

  const cats: Category[] = CATEGORIES.filter(c => topics.some(t => t.cat === c))

  // These categories stay open; the rest collapse to keep the rail short, and
  // behave as an accordion (opening one closes the previously-open collapsible).
  const alwaysOpen = new Set<Category>(['Get started', 'Receiving', 'Sending', 'Wallets & devices'])
  let openCollapsible = $state<Category | null>(null)
  const catOpen = (cat: Category) => alwaysOpen.has(cat) || openCollapsible === cat
  function toggleCat(cat: Category) {
    if (alwaysOpen.has(cat)) return
    openCollapsible = openCollapsible === cat ? null : cat
  }

  // Hand-picked next-steps shown at the foot of each article.
  const relatedMap: Record<string, string[]> = {
    welcome:          ['choosing-setup', 'add-wallet', 'connect-backend'],
    'choosing-setup': ['wallet-kinds', 'connect-backend', 'multisig-setup'],
    'add-wallet':     ['connect-backend', 'wallet-kinds', 'seed'],
    'connect-backend':['add-wallet', 'receive'],
    receive:          ['send', 'addresses', 'sp-setup'],
    'sp-setup':       ['sp-concept', 'receive'],
    send:             ['fees', 'speed-up', 'warnings'],
    'speed-up':       ['fees', 'send'],
    'coin-control':   ['spending-privately', 'warnings', 'coins-concept'],
    'spending-privately': ['coin-control', 'payjoin-send', 'sp-setup'],
    'payjoin-send':   ['payjoin', 'warnings'],
    'payjoin-receive':['payjoin', 'receive'],
    payjoin:          ['payjoin-send', 'payjoin-receive'],
    warnings:         ['spending-privately', 'coin-control', 'payjoin'],
    encryption:       ['backup', 'seed', 'spending-privately'],
    'multisig-setup': ['wallet-kinds', 'hardware'],
    hardware:         ['send', 'add-wallet'],
    sweep:            ['add-wallet', 'send'],
    backup:           ['seed', 'import-export', 'encryption'],
    labels:           ['coin-control', 'import-export'],
    tax:              ['backup', 'import-export'],
    'import-export':  ['backup', 'labels'],
    broadcast:        ['psbt-inspector', 'send'],
    'psbt-inspector': ['broadcast', 'send'],
    consolidate:      ['coin-control', 'fees'],
    messages:         ['address-lookup', 'seed'],
    'address-lookup': ['addresses', 'messages'],
    'export-keys':    ['backup', 'wallet-kinds'],
    troubleshooting:  ['connect-backend', 'hardware', 'backup'],
    'wallet-kinds':   ['add-wallet', 'seed'],
    addresses:        ['receive', 'coins-concept'],
    seed:             ['backup', 'add-wallet'],
    'sp-concept':     ['sp-setup', 'wallet-kinds'],
    'sending-concept':['send', 'fees'],
    fees:             ['send', 'speed-up'],
    'coins-concept':  ['coin-control', 'fees'],
  }
  const titleOf = (id: string) => topics.find(t => t.id === id)?.title ?? id

  let active = $state('welcome')
  let query = $state('')
  let related = $derived(relatedMap[active] ?? [])

  // Search index, built once in onMount from the rendered article sections so
  // we never keep a second copy of the prose. Invariant: HelpContent must render
  // every topic as a `.article` element carrying a matching `data-id`, and all
  // of them must be present at mount — inactive ones use `hidden`, not
  // conditional rendering. Lazy-rendering sections would silently break search.
  let index = $state<{ id: string; text: string }[]>([])
  let contentEl: HTMLElement
  let searchEl: HTMLInputElement | undefined

  type Snip = { pre: string; hit: string; post: string }
  type Result = { id: string; title: string; snip: Snip; score: number }

  function makeSnippet(text: string, q: string, bi: number): Snip {
    if (bi < 0) return { pre: text.slice(0, 150), hit: '', post: '' }
    let start = Math.max(0, bi - 48)
    while (start > 0 && text[start] !== ' ') start--
    let end = Math.min(text.length, bi + q.length + 90)
    while (end < text.length && text[end] !== ' ') end++
    return {
      pre: text.slice(start, bi).trimStart(),
      hit: text.slice(bi, bi + q.length),
      post: text.slice(bi + q.length, end).trimEnd(),
    }
  }

  let results = $derived.by<Result[]>(() => {
    const q = query.trim().toLowerCase()
    if (!q) return []
    const out: Result[] = []
    for (const t of topics) {
      const text = index.find(e => e.id === t.id)?.text ?? ''
      const inTitle = t.title.toLowerCase().includes(q)
      const inKw = t.keywords.includes(q)
      const bi = text.toLowerCase().indexOf(q)
      if (!inTitle && !inKw && bi < 0) continue
      out.push({ id: t.id, title: t.title, snip: makeSnippet(text, q, bi), score: inTitle ? 0 : bi >= 0 ? 1 : 2 })
    }
    return out.sort((a, b) => a.score - b.score)
  })

  function select(id: string) {
    active = id
    query = ''
    const cat = topics.find(t => t.id === id)?.cat
    if (cat && !alwaysOpen.has(cat)) openCollapsible = cat
    history.replaceState(null, '', `#${id}`)
  }

  function applyHash() {
    const id = location.hash.replace(/^#/, '')
    const t = topics.find(tp => tp.id === id)
    if (!t) return
    active = id
    if (!alwaysOpen.has(t.cat)) openCollapsible = t.cat
  }

  function onKey(e: KeyboardEvent) {
    const inField = document.activeElement instanceof HTMLInputElement
    if (e.key === '/' && !inField) { e.preventDefault(); searchEl?.focus() }
    else if (e.key === 'Escape' && document.activeElement === searchEl) { query = ''; searchEl?.blur() }
  }

  // Dev-only integrity check: the topic list, the rendered sections, and the
  // related map are hand-synced across files, so shout the moment they drift
  // instead of failing silently (a blank pane or a dead-end "Related" chip).
  function checkIntegrity() {
    const ids = new Set(topics.map(t => t.id))
    const sectionIds = new Set(index.map(e => e.id))
    for (const id of ids) if (!sectionIds.has(id)) console.error(`[help] topic "${id}" has no <section data-id="${id}"> in HelpContent`)
    for (const e of index) if (!ids.has(e.id)) console.error(`[help] <section data-id="${e.id}"> has no topic entry`)
    for (const [k, vs] of Object.entries(relatedMap)) {
      if (!ids.has(k)) console.error(`[help] relatedMap key "${k}" is not a topic id`)
      for (const v of vs) if (!ids.has(v)) console.error(`[help] relatedMap["${k}"] references unknown topic "${v}"`)
    }
  }

  onMount(() => {
    applyHash()
    index = Array.from(contentEl.querySelectorAll<HTMLElement>('.article')).map(el => ({
      id: el.dataset.id!,
      text: (el.textContent ?? '').replace(/\s+/g, ' ').trim(),
    }))
    if (import.meta.env.DEV) checkIntegrity()
    window.addEventListener('hashchange', applyHash)
    window.addEventListener('keydown', onKey)
  })
  onDestroy(() => {
    if (typeof window === 'undefined') return
    window.removeEventListener('hashchange', applyHash)
    window.removeEventListener('keydown', onKey)
  })
</script>

<div class="help">
  <nav class="rail" aria-label="Help topics">
    <div class="search-wrap">
      <input
        class="search"
        type="search"
        placeholder="Search help"
        bind:value={query}
        bind:this={searchEl}
        aria-label="Search help"
      />
    </div>
    <div class="rail-scroll">
      {#if query.trim()}
        {#each results as r (r.id)}
          <button class="result" onclick={() => select(r.id)}>
            <span class="result-title">{r.title}</span>
            <span class="result-snip">{r.snip.pre}{#if r.snip.hit}<mark>{r.snip.hit}</mark>{/if}{r.snip.post}</span>
          </button>
        {/each}
        {#if results.length === 0}
          <p class="rail-empty">No matches for “{query.trim()}”.</p>
        {/if}
      {:else}
        {#each cats as cat (cat)}
          {@const open = catOpen(cat)}
          <div class="rail-group">
            {#if alwaysOpen.has(cat)}
              <div class="rail-cat">{cat}</div>
            {:else}
              <button class="rail-cat rail-cat-btn" aria-expanded={open} onclick={() => toggleCat(cat)}>
                <span class="cat-chevron" class:open>▸</span>
                {cat}
              </button>
            {/if}
            {#if open}
              {#each topics.filter(t => t.cat === cat) as t (t.id)}
                <button class="rail-item" class:active={active === t.id} onclick={() => select(t.id)}>
                  {t.title}
                </button>
              {/each}
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </nav>

  <article class="content" tabindex="-1">
    <div class="content-inner" bind:this={contentEl}>
      <HelpContent {active} {select} />

      {#if related.length}
        <nav class="related" aria-label="Related topics">
          <span class="related-label">Related</span>
          <div class="related-chips">
            {#each related as rid (rid)}
              <button class="chip" onclick={() => select(rid)}>{titleOf(rid)}</button>
            {/each}
          </div>
        </nav>
      {/if}
    </div>
  </article>
</div>

<style>
  .help {
    flex: 1;
    display: flex;
    min-width: 0;
    overflow: hidden;
    background: var(--surface-2);
  }

  /* Left rail: its own scroll, never the whole page. */
  .rail {
    width: 232px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    background: var(--surface-1);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .search-wrap { padding: 14px 12px 8px; border-bottom: 1px solid var(--border); }
  .search {
    width: 100%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 7px 10px;
    color: var(--text);
    font-size: var(--text-sm);
  }
  .search::placeholder { color: var(--text-muted); }
  .search:focus { outline: none; border-color: var(--accent); }

  .rail-scroll { flex: 1; overflow-y: auto; padding: 8px 0 16px; }
  .rail-group { margin-bottom: 4px; }
  .rail-cat {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    padding: 12px 16px 4px;
    font-weight: 600;
  }
  .rail-cat-btn {
    width: 100%;
    font-family: inherit;
    background: none;
    border: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .rail-cat-btn:hover { color: var(--text); }
  .cat-chevron {
    font-size: 0.6rem;
    line-height: 1;
    color: var(--text-muted);
    transition: transform 0.12s;
  }
  .cat-chevron.open { transform: rotate(90deg); }

  .rail-item {
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    padding: 7px 16px 7px 13px;
    cursor: pointer;
    color: var(--text);
    font-size: var(--text-sm);
    display: block;
  }
  .rail-item:hover { background: var(--surface-hover); }
  .rail-item.active {
    background: var(--surface-active);
    border-left-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .rail-empty { color: var(--text-muted); font-size: var(--text-sm); padding: 16px; }

  .result {
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-left: 3px solid transparent;
    padding: 9px 16px 10px 13px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .result:hover { background: var(--surface-hover); }
  .result-title { font-size: var(--text-sm); font-weight: 600; color: var(--text); }
  .result-snip {
    font-size: var(--text-xs);
    color: var(--text-muted);
    line-height: 1.45;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .result-snip :global(mark) {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--text);
    border-radius: 2px;
  }

  /* Right pane: its own scroll, one topic at a time. */
  .content { flex: 1; overflow-y: auto; min-width: 0; }
  .content:focus { outline: none; }
  .content-inner {
    max-width: 720px;
    margin: 0 auto;
    padding: 36px 48px 64px;
  }

  .related {
    margin-top: 44px;
    padding-top: 20px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .related-label {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    font-weight: 600;
  }
  .related-chips { display: flex; flex-wrap: wrap; gap: 8px; }
  .chip {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: 20px;
    padding: 6px 14px;
    font-size: var(--text-sm);
    color: var(--text);
    cursor: pointer;
  }
  .chip:hover { border-color: var(--accent); color: var(--accent); }

  @media (max-width: 768px) {
    .help { flex-direction: column; overflow-y: auto; }
    .rail {
      width: 100%;
      border-right: none;
      border-bottom: 1px solid var(--border);
      max-height: 42vh;
    }
    .content { overflow-y: visible; }
    .content-inner { padding: 24px 18px 48px; }
  }
</style>
