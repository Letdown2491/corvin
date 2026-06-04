<script lang="ts">
  // The 5-tile wallet-kind picker shown first in the add-wallet flow. Purely
  // presentational — emits the chosen kind to the parent.
  type WalletKind = 'single' | 'multisig' | 'sp' | 'address' | 'vault'
  let { onPick }: { onPick: (k: WalletKind) => void } = $props()
</script>

<div class="kind-grid">
  <button type="button" class="kind-tile kind-tile-primary" onclick={() => onPick('single')}>
    <span class="kind-tile-pill">Most Common</span>
    <span class="kind-tile-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="8" cy="15" r="4"/>
        <path d="M11 12 L20 3"/>
        <path d="M17 6 L19 8"/>
        <path d="M14 9 L16 11"/>
      </svg>
    </span>
    <span class="kind-tile-title">Single Signature</span>
    <span class="kind-tile-desc">
      One signer. Generate a new seed, import an existing one, or connect a hardware wallet.
    </span>
    <span class="kind-tile-arrow" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"/>
        <path d="M13 5l7 7-7 7"/>
      </svg>
    </span>
  </button>
  <button type="button" class="kind-tile" onclick={() => onPick('multisig')}>
    <span class="kind-tile-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="7" cy="8" r="3"/>
        <circle cx="17" cy="8" r="3"/>
        <circle cx="12" cy="17" r="3"/>
        <path d="M9.5 10 L12 14"/>
        <path d="M14.5 10 L12 14"/>
      </svg>
    </span>
    <span class="kind-tile-title">Multisig</span>
    <span class="kind-tile-desc">
      m-of-n with separate signers. Each contributes a seed, hardware wallet, or xpub.
    </span>
    <span class="kind-tile-arrow" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"/>
        <path d="M13 5l7 7-7 7"/>
      </svg>
    </span>
  </button>
  <button type="button" class="kind-tile" onclick={() => onPick('sp')}>
    <span class="kind-tile-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M12 3 L20 7.5 V16.5 L12 21 L4 16.5 V7.5 Z"/>
        <circle cx="12" cy="12" r="2.5" fill="currentColor" stroke="none"/>
      </svg>
    </span>
    <span class="kind-tile-title">Silent Payments</span>
    <span class="kind-tile-desc">
      One reusable address; senders derive a fresh destination per payment. Needs an SP-capable Electrum server.
    </span>
    <span class="kind-tile-arrow" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"/>
        <path d="M13 5l7 7-7 7"/>
      </svg>
    </span>
  </button>
  <button type="button" class="kind-tile" onclick={() => onPick('address')}>
    <span class="kind-tile-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <path d="M2 12 C5 6, 8 5, 12 5 S19 6, 22 12 C19 18, 16 19, 12 19 S5 18, 2 12 Z"/>
        <circle cx="12" cy="12" r="3"/>
      </svg>
    </span>
    <span class="kind-tile-title">Watch-only</span>
    <span class="kind-tile-desc">
      A static address you can monitor but not spend from. For observation or shared use.
    </span>
    <span class="kind-tile-arrow" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"/>
        <path d="M13 5l7 7-7 7"/>
      </svg>
    </span>
  </button>
  <button type="button" class="kind-tile" onclick={() => onPick('vault')}>
    <span class="kind-tile-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        <rect x="4" y="10" width="16" height="11" rx="2"/>
        <path d="M8 10V7a4 4 0 0 1 8 0v3"/>
        <circle cx="12" cy="15" r="1.5" fill="currentColor" stroke="none"/>
      </svg>
    </span>
    <span class="kind-tile-title">Vault / Policy</span>
    <span class="kind-tile-desc">
      Timelock templates: an inheritance vault (recovery group after a delay), or savings locked until a future date.
    </span>
    <span class="kind-tile-arrow" aria-hidden="true">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M5 12h14"/>
        <path d="M13 5l7 7-7 7"/>
      </svg>
    </span>
  </button>
</div>

<style>
  .kind-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    grid-auto-rows: 1fr;
    gap: 12px;
    margin-top: 8px;
  }
  .kind-tile {
    position: relative;
    background: var(--surface-2); border: 1px solid var(--border); border-radius: 8px;
    padding: 20px 22px 18px; cursor: pointer; text-align: left;
    display: flex; flex-direction: column; gap: 10px;
    transition: border-color 0.15s, background 0.15s, box-shadow 0.15s;
    color: var(--text);
    min-height: 148px;
    overflow: hidden;
  }
  .kind-tile:hover {
    border-color: var(--accent);
    background: var(--surface-active);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .kind-tile-primary {
    /* Subtle accent border so the recommended path is findable at a glance,
       without overwhelming the other tiles. */
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  .kind-tile-pill {
    position: absolute; top: 10px; right: 12px;
    font-size: 0.66rem; font-weight: 700; letter-spacing: 0.05em;
    text-transform: uppercase;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 35%, transparent);
    border-radius: 999px;
    padding: 2px 8px;
    line-height: 1.4;
  }
  .kind-tile-icon {
    display: inline-flex; align-items: center; justify-content: center;
    width: 36px; height: 36px; border-radius: 8px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--accent);
    flex-shrink: 0;
  }
  .kind-tile-icon svg { width: 20px; height: 20px; }
  .kind-tile-title {
    font-size: 1rem; font-weight: 600; color: var(--text);
    margin-top: 2px;
  }
  .kind-tile-desc {
    font-size: 0.82rem; color: var(--text-muted); line-height: 1.5;
    flex: 1;
  }
  .kind-tile-arrow {
    position: absolute; bottom: 14px; right: 14px;
    width: 16px; height: 16px;
    color: var(--text-muted);
    opacity: 0.35;
    transition: opacity 0.15s, color 0.15s, transform 0.15s;
  }
  .kind-tile-arrow svg { width: 100%; height: 100%; }
  .kind-tile:hover .kind-tile-arrow {
    opacity: 1;
    color: var(--accent);
    transform: translateX(2px);
  }
  @media (max-width: 820px) {
    .kind-grid { grid-template-columns: 1fr 1fr; }
  }
  @media (max-width: 560px) {
    .kind-grid { grid-template-columns: 1fr; grid-auto-rows: auto; }
    .kind-tile { min-height: 0; }
  }
</style>
