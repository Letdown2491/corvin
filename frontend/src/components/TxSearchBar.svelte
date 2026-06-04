<script lang="ts">
  // Transaction search input + its collapsible syntax-help panel. The parent owns
  // the query string (and parses it); this component only renders + toggles help.
  let { query = $bindable(), summary }: { query: string; summary: string | null } = $props()

  let helpOpen = $state(false)
</script>

<div class="search-row">
  <label for="search-txs" class="sr-only">Search transactions</label>
  <div class="search-input-wrap">
    <input
      id="search-txs"
      class="search-input"
      type="search"
      placeholder="Search transactions…"
      bind:value={query}
    />
    <button
      class="search-help-btn"
      class:active={helpOpen}
      onclick={() => helpOpen = !helpOpen}
      title="Search syntax help"
      aria-expanded={helpOpen}
      aria-controls="search-help-panel"
    >?</button>
  </div>
  {#if summary}
    <p class="search-hint">Showing: {summary}</p>
  {/if}
  {#if helpOpen}
    <div class="search-help" id="search-help-panel">
      <div class="help-group">
        <span class="help-heading">Direction</span>
        <code>sent</code><code>received</code>
      </div>
      <div class="help-group">
        <span class="help-heading">Status</span>
        <code>unconfirmed</code><code>confirmed</code>
      </div>
      <div class="help-group">
        <span class="help-heading">Amount</span>
        <code>more than 0.1 btc</code><code>under 50000 sats</code><code>between 0.01 and 0.5 btc</code>
      </div>
      <div class="help-group">
        <span class="help-heading">Date</span>
        <code>in january</code><code>last month</code><code>q1 2024</code><code>last 30 days</code><code>this year</code>
      </div>
      <div class="help-group">
        <span class="help-heading">Labels</span>
        <code>label coffee</code><code>has label</code>
      </div>
      <div class="help-group">
        <span class="help-heading">Combined</span>
        <code>sent in march more than 0.1 btc</code>
      </div>
    </div>
  {/if}
</div>

<style>
  .search-row { margin-bottom: 10px; display: flex; flex-direction: column; gap: 5px; }
  .search-input-wrap { display: flex; gap: 5px; align-items: center; max-width: 420px; }
  .search-input {
    flex: 1;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text); padding: 6px 10px; font-size: 0.82rem;
  }
  .search-input:focus { outline: 1px solid var(--accent); }
  .search-help-btn {
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 5px;
    color: var(--text-muted); cursor: pointer; font-size: 0.78rem; font-weight: 700;
    padding: 5px 9px; flex-shrink: 0; transition: color 0.15s, border-color 0.15s;
  }
  .search-help-btn:hover, .search-help-btn.active {
    color: var(--accent); border-color: var(--accent);
  }
  .search-hint { margin: 0; font-size: 0.72rem; color: var(--accent); opacity: 0.85; }
  .search-help {
    max-width: 520px;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 6px;
    padding: 12px 14px; display: flex; flex-direction: column; gap: 8px;
  }
  .help-group { display: flex; flex-wrap: wrap; align-items: baseline; gap: 5px; }
  .help-heading {
    font-size: 0.67rem; font-weight: 700; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--text-muted); min-width: 70px; flex-shrink: 0;
  }
  .help-group code {
    font-family: monospace; font-size: 0.72rem;
    background: var(--surface-1); border: 1px solid var(--border);
    border-radius: 3px; padding: 1px 5px; color: var(--text);
  }

  @media (max-width: 768px) {
    .search-input-wrap { max-width: 100%; }
  }
</style>
