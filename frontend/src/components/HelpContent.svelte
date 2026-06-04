<script lang="ts">
  // The help article prose, split out from HelpPage so copy can be edited
  // without wading through the rail/search logic. Every article is a
  // `.article` with a `data-id` — HelpPage's search index depends on that.
  let { active, select }: { active: string; select: (id: string) => void } = $props()
</script>

<!-- Internal cross-link: one id drives both the href and the in-page nav. -->
{#snippet xref(id: string, label: string)}<a href="#{id}" onclick={() => select(id)}>{label}</a>{/snippet}

<!-- ───────────────────────── Get started ───────────────────────── -->
<section class="article" data-id="welcome" hidden={active !== 'welcome'}>
  <h1>What is Corvin?</h1>
  <p>Corvin is a Bitcoin wallet you run yourself. It's a single program on your own machine, and it reaches the Bitcoin network through a server you pick. That can be your own node, or another one you trust. There's no account, no sign-up, and no company sitting between you and your coins.</p>
  <p>Because it runs locally, nothing about your wallet leaves this machine unless you send it somewhere on purpose. The trade-off is that you're in charge. If you lose your recovery phrase, no one can reset it for you.</p>
  <p>Two things get you running: {@render xref('connect-backend', 'connect a backend')} so Corvin can see the chain, then {@render xref('add-wallet', 'add a wallet')}. The guides on the left walk each task step by step. The Concepts section explains the ideas behind them, and none of it is loaded from the internet. It ships inside the app.</p>
</section>

<section class="article" data-id="choosing-setup" hidden={active !== 'choosing-setup'}>
  <h1>Choosing your setup</h1>
  <p>Corvin can be a quick everyday wallet or a hardened vault for long-term savings. There is no single right answer. The setup that fits you depends on what the coins are for, how much privacy you want, and how much effort you're willing to spend. You can change your mind later, and you can run more than one wallet at once.</p>

  <h2>Match the wallet to the job</h2>
  <p>For coins you spend often, a <strong>single-signature</strong> wallet is simple and fast. Generate a seed, write it down, and you're ready. For savings you rarely touch, consider a <strong>hardware wallet</strong> so the keys never reach this computer, or a <strong>multisig</strong> wallet that needs several devices to sign, so losing or compromising one of them isn't fatal. {@render xref('wallet-kinds', 'Wallet kinds explained')} covers each option, and {@render xref('multisig-setup', 'multisig')} and {@render xref('hardware', 'hardware wallets')} walk through setup.</p>
  <p>A common pattern is two wallets: a small single-sig wallet for spending, and a separate hardware or multisig wallet for savings you move into rarely.</p>

  <h2>Decide how private you want to be</h2>
  <p>The server you connect to is the biggest privacy lever. A <strong>public Electrum server</strong> is convenient, but whoever runs it can see the addresses you're watching. Connecting to <strong>your own</strong> Electrum server or Bitcoin node keeps that to yourself. See {@render xref('connect-backend', 'Connect a backend')}.</p>
  <p>From there, Corvin gives you tools to avoid linking your coins together: {@render xref('coin-control', 'coin control')} to choose which coins you spend, {@render xref('labels', 'labels')} to keep them straight, and {@render xref('warnings', 'privacy warnings')} that flag a risky spend before you sign. {@render xref('spending-privately', 'Spending privately')} ties these together. To receive without reusing an address, {@render xref('sp-setup', 'silent payments')} give you one reusable address that doesn't link your payments.</p>

  <h2>Protect the backup</h2>
  <p>Whatever you choose, the recovery phrase is what actually holds your coins. Write it on paper, keep it offline, and never type it into anything but Corvin. For larger amounts, keep more than one copy in separate places. {@render xref('seed', 'Recovery phrases')} and {@render xref('backup', 'Back up and restore')} go deeper.</p>
</section>

<section class="article" data-id="add-wallet" hidden={active !== 'add-wallet'}>
  <h1>Add your first wallet</h1>
  <p>Corvin can hold as many wallets as you like. Adding one always starts the same way, and the kind you choose decides the rest of the form.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Decide which kind you want. For an everyday wallet, that's Single Signature. If you're not sure, read {@render xref('wallet-kinds', 'Wallet kinds explained')} first.</div>
  <ol>
    <li>In the sidebar, click <strong>Add wallet</strong>.</li>
    <li>Pick a kind from the tiles: <strong>Single Signature</strong>, <strong>Multisig</strong>, <strong>Silent Payments</strong>, <strong>Watch-only</strong>, or <strong>Vault / Policy</strong>. For a normal wallet, choose Single Signature.</li>
    <li>Give the wallet a <strong>Name</strong>, such as “Spending”.</li>
    <li>Choose how the keys come in using the tabs: <strong>Seed phrase</strong>, <strong>Paste xpub</strong>, <strong>Import file</strong>, <strong>Hardware wallet</strong>, or <strong>Descriptor</strong>.</li>
    <li>For a brand-new wallet, keep <strong>Seed phrase</strong> on <strong>Generate new</strong>. Corvin shows the words. Write them down, tick the confirmation, then pass the short three-word check. This is the one step worth slowing down for, and {@render xref('seed', 'Recovery phrases')} explains why.</li>
    <li>Optionally set a BIP39 passphrase. Leave it blank if you're unsure.</li>
    <li>Click <strong>Add wallet</strong> at the bottom.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the wallet appears in the sidebar and opens to its dashboard. Pasting only an address or an xpub instead makes a <strong>watch-only</strong> wallet: it can receive and show balances, but it can't sign, so it can't send.</p>
</section>

<section class="article" data-id="connect-backend" hidden={active !== 'connect-backend'}>
  <h1>Connect a backend</h1>
  <p>Corvin doesn't reach Bitcoin on its own. It connects to a server you choose, and that's what lets it show balances, build transactions, and broadcast them. You set a <strong>default backend</strong> on the Backend page, and any wallet can use its own instead.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Corvin ships with a public Electrum server selected, so it works straight away. To use your own server, have its details ready: a host and port for your Electrum server, or the URL and credentials for your Bitcoin node (Bitcoin Core, Knots, etc.).</div>
  <ol>
    <li>In the sidebar, click <strong>Backend</strong>. The page is a stack of sections — <strong>Network</strong>, <strong>Default backend</strong>, <strong>Saved backends</strong>, <strong>Mempool</strong>, <strong>Payjoin</strong> — and everything <strong>saves automatically</strong> as you change it (a small "Autosaved ✓" appears in the section header — there's no Save button).</li>
    <li>Open <strong>Network</strong> and set it. Most people want Mainnet. One instance runs one network, and switching hides wallets that belong to the other network until you switch back (restart to fully apply).</li>
    <li>In <strong>Default backend</strong>, pick a server. The list offers a few public Electrum servers plus any backends you've saved. This is what wallets use unless you give them their own.</li>
    <li>To add your own (private Electrum, or your Bitcoin node), open <strong>Saved backends</strong>, click add, choose the type, enter its details, and <strong>Test connection</strong>. It then appears in the Default-backend list and in the per-wallet picker.</li>
  </ol>
  <p>Which server to use comes down to privacy. A <strong>public Electrum server</strong> is the quickest way to start, but whoever runs it can see which addresses you're watching. Running <strong>your own</strong> Electrum server (such as electrs or Fulcrum alongside your node) or connecting to your own <strong>Bitcoin node</strong> keeps that private, and is the better choice as your holdings grow. {@render xref('choosing-setup', 'Choosing your setup')} weighs this up.</p>
  <p><strong>Per-wallet servers.</strong> You don't have to send everything through one server. When you add a wallet you can pin it to any saved backend, so a low-stakes spending wallet can stay on a public server while your savings wallet talks only to your own node — and neither server sees both. To change a wallet's server later, open the wallet's <strong>⋯ menu → Change backend</strong>.</p>
  <p class="outcome"><strong>Result:</strong> each wallet's page shows its server, connection state, and the current block; the sidebar shows a small status dot per wallet. If a server can't be reached, that wallet's status turns red with the error. You can also set a mempool URL here, which turns on fee estimates and price data.</p>
</section>

<section class="article" data-id="offline" hidden={active !== 'offline'}>
  <h1>Use Corvin offline</h1>
  <p>Corvin has an <strong>offline mode</strong> for air-gapped use: it never opens a connection to any backend. You can still add wallets, see your last-synced balances and history, and — most importantly — <strong>import, sign, and export transactions</strong>. Only syncing and broadcasting are unavailable, since both need the network. It turns Corvin into a dedicated signing device.</p>
  <div class="callout"><span class="callout-label">Honest about air-gaps</span> Offline mode stops <em>Corvin</em> from reaching the network. A true air-gap also means keeping the <em>machine</em> itself offline — the toggle is one half of that.</div>
  <ol>
    <li>Open <strong>Backend → Network</strong> and tick <strong>Offline mode</strong>. An "Offline mode" badge appears in the sidebar and the connection dots go neutral.</li>
    <li>Add the wallets you want to sign with — a watch-only descriptor, an xpub, or a seed. No backend is needed to add them or to show their addresses.</li>
    <li>Bring an <strong>unsigned transaction</strong> over from your online machine: build it there on a watch-only copy of the same wallet, then move the PSBT across by file or QR.</li>
    <li>Open it (the kebab <strong>⋯ → Broadcast</strong> or <strong>Inspect PSBT</strong> tool, or load it in the send flow), sign it, and on the verify screen choose <strong>Export signed transaction</strong>.</li>
    <li>Carry the signed transaction back to an online machine and broadcast it there ({@render xref('broadcast', 'Broadcast a transaction')}).</li>
  </ol>
  <p>This is the classic cold-storage split: an online, watch-only wallet builds and broadcasts; an offline signer holds the keys and never touches the network. The {@render xref('hardware', 'hardware wallet')} and QR flows fit the same pattern. To go back online, untick <strong>Offline mode</strong> — Corvin reconnects and resumes syncing.</p>
  <p class="outcome"><strong>Result:</strong> a wallet that can sign and export without ever connecting. See {@render xref('send', 'Send a payment')} for the signing steps and {@render xref('sending-concept', 'How sending works')} for the PSBT picture.</p>
</section>

<!-- ───────────────────────── Receiving ───────────────────────── -->
<section class="article" data-id="receive" hidden={active !== 'receive'}>
  <h1>Receive a payment</h1>
  <p>Receiving means handing the sender an address. Corvin gives you a fresh one for each request so your payments don't link together on the blockchain.</p>
  <ol>
    <li>Open the wallet and click <strong>Receive</strong>.</li>
    <li>Corvin shows your next unused address as text and as a QR code. Give either one to the sender.</li>
    <li>Optionally enter an <strong>amount</strong> and a <strong>label</strong>. That builds a payment request the sender's wallet can read, with the amount filled in for them.</li>
    <li>If the wallet is backed by a hardware device, you can verify the address on the device before sharing it, so you know it's really yours.</li>
  </ol>
  <p>A silent-payments wallet works differently here. Instead of a new address each time, it shows one reusable <code>sp1q…</code> address you can copy and hand out repeatedly. You can also add labelled variants of it.</p>
  <p class="outcome"><strong>Result:</strong> once the sender pays, the transaction appears in your history after the next sync. Each new request shows a different address on purpose, which is covered in {@render xref('addresses', 'Addresses')}.</p>
</section>

<section class="article" data-id="sp-setup" hidden={active !== 'sp-setup'}>
  <h1>Set up silent payments</h1>
  <p>Silent payments is its own kind of wallet, so you create it from the Add wallet screen rather than switching an existing wallet over.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Silent payments scanning needs a Frigate (BIP-352) server. Corvin offers a public one (<code>frigate.2140.dev</code>); to scan on your own, add a <strong>Frigate</strong> backend under Backend → Saved backends first.</div>
  <ol>
    <li>Click <strong>Add wallet</strong>, then the <strong>Silent Payments</strong> tile.</li>
    <li>Give the wallet a name.</li>
    <li>Choose its <strong>scanner</strong>: the public Frigate server, or a Frigate backend you've saved (to keep this wallet's receipts private to your own server).</li>
    <li>Choose <strong>From seed</strong> to create or import a wallet you control, or <strong>Watch-only</strong> to monitor someone else's <code>sp1q…</code> address.</li>
    <li>From seed: keep <strong>Generate new</strong> (write the words down and pass the check), or pick <strong>Import existing</strong> and paste your phrase. You can set an account number and a passphrase if you need them.</li>
    <li>Watch-only: paste the 64-character <strong>scan secret</strong> and the 66-character <strong>spend pubkey</strong>. Optionally set a birthday block height so the scanner can skip older blocks.</li>
    <li>Click <strong>Add wallet</strong>.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the wallet starts scanning in the background, and incoming payments appear as the scanner finds them. One thing to know going in: the scan server can see what you receive during the session. {@render xref('sp-concept', 'How silent payments work')} explains that trade-off.</p>
</section>

<!-- ───────────────────────── Sending ───────────────────────── -->
<section class="article" data-id="send" hidden={active !== 'send'}>
  <h1>Send a payment</h1>
  <p>Sending happens in two parts: Corvin builds the transaction, then something signs it. The Send screen walks both.</p>
  <div class="callout"><span class="callout-label">Before you start</span> You need a funded wallet that can sign, and the recipient's address.</div>
  <ol>
    <li>Open the wallet and click <strong>Send</strong>.</li>
    <li>Under <strong>Spend via</strong>, keep the normal path. The silent-payments option is for paying an <code>sp1</code> address.</li>
    <li>In <strong>Recipients</strong>, paste the address and enter the amount. Use <strong>Add recipient</strong> to pay several people at once, and switch the unit between sats and BTC if you prefer. Corvin echoes the address back in small groups of characters so you can check it against the one you were given. Compare it carefully, since clipboard-swapping malware works by silently replacing a pasted address.</li>
    <li>You can also type a <strong>₿name@domain</strong> (a BIP-353 human-readable name) instead of an address. Corvin resolves it through a DNSSEC-validated DNS lookup, so the answer can't be forged. Be aware that the lookup reveals the name to your DNS resolver, which can see who you're about to pay. You can change that resolver under Backend settings, Name resolution (a SOCKS5 proxy hides your IP from it, but not the name).</li>
    <li>Pick a <strong>Fee rate</strong>: slow, medium, fast, or <strong>Custom</strong> for an exact sat/vB. {@render xref('fees', 'Fees and sat/vB')} explains the choice.</li>
    <li>Optionally open <strong>Coin control</strong> to choose exactly which coins to spend.</li>
    <li>Click <strong>Preview transaction</strong>. Corvin builds the transaction and moves to a review screen so you check the whole thing before anything is signed.</li>
    <li>On the review screen, read the <strong>flow diagram</strong>, which shows your inputs splitting into the recipient, your change, and the fee as one proportional bar, and read any privacy warnings. Corvin also flags sanity checks here, such as a fee that's large next to the amount or a send that empties most of the wallet. These are reminders, not blocks. Use <strong>Edit transaction</strong> to go back if something's off.</li>
    <li>Confirm the recipient and amount one last time, then sign using the footer buttons: <strong>Hardware wallet</strong>, <strong>QR code</strong> for an air-gapped device, or <strong>Export PSBT</strong> / <strong>Copy PSBT</strong> for an external signer. A software wallet signs in place.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the signed transaction is broadcast and shows in your history as pending. If you want the picture behind these steps, see {@render xref('sending-concept', 'How sending works')}.</p>
</section>

<section class="article" data-id="speed-up" hidden={active !== 'speed-up'}>
  <h1>Speed up a stuck payment</h1>
  <p>If a transaction is sitting unconfirmed because the fee was too low, you can raise it. Corvin offers two methods and picks whichever fits the transaction.</p>
  <div class="callout"><span class="callout-label">Before you start</span> You need an unconfirmed transaction in your history.</div>
  <ol>
    <li>Open the stuck transaction from the history list.</li>
    <li>Start a fee bump. Use <strong>Replace-by-Fee (RBF)</strong> for a payment you sent, or <strong>Child-Pays-for-Parent (CPFP)</strong> for one you're receiving or otherwise can't replace.</li>
    <li>Set a higher <strong>New fee rate</strong>, either a preset or a custom sat/vB.</li>
    <li>Check that the recipient and amount are unchanged, then sign the new transaction the same way you sign a send.</li>
    <li>Broadcast it.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the higher-fee transaction goes out. With RBF it replaces the original. With CPFP it drags the original along by making the pair worth confirming together. A rule of thumb: replace what you sent, child-pays what you're waiting on.</p>
</section>

<section class="article" data-id="coin-control" hidden={active !== 'coin-control'}>
  <h1>Use coin control</h1>
  <p>Your balance is made of separate coins. Coin control lets you choose which ones a payment spends, and keep coins of different origins apart. Most of the time you can let Corvin pick, but when fees or privacy matter, it's worth steering.</p>
  <ol>
    <li>Open the wallet's coins list to see each unspent coin on its own.</li>
    <li>To annotate a coin, add a <strong>note</strong> or a coloured <strong>category</strong> to it (or to the address that received it). Both stay on your machine and travel with your backup. See {@render xref('labels', 'Notes and categories')}.</li>
    <li>To set a coin aside, <strong>freeze</strong> it. Corvin won't spend a frozen coin automatically, and you can unfreeze it anytime.</li>
    <li>When sending, open the <strong>Coin control</strong> section and tick exactly the coins you want to spend. Each coin shows its category chip there, so you can keep compartments apart and avoid the mixed-category warning.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> your payment uses only the coins you chose, which helps both the fee and your privacy. The reasoning is in {@render xref('coins-concept', 'Coins, UTXOs, and change')} and {@render xref('warnings', 'Privacy warnings')}.</p>
</section>

<!-- ───────────────────────── Wallets & devices ───────────────────────── -->
<section class="article" data-id="multisig-setup" hidden={active !== 'multisig-setup'}>
  <h1>Set up a multisig wallet</h1>
  <p>A multisig wallet needs several signers to agree before it can spend, such as two of three. Setting one up means collecting each signer's public key. See {@render xref('wallet-kinds', 'Wallet kinds explained')} for when this is worth the extra effort.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Gather each signer's public key (an xpub with its origin info) or the devices to connect, so you can finish in one sitting. Corvin saves your progress if you step away, but it's smoother in one go.</div>
  <ol>
    <li>Click <strong>Add wallet</strong>, choose the <strong>Multisig</strong> tile, and give it a name.</li>
    <li>Set the threshold and total, for example 2 of 3.</li>
    <li>Fill in each signer card. For each one you can connect a hardware device, import a file (Coldcard, Sparrow, Specter), or paste an xpub with its <code>[fingerprint/path]</code> origin.</li>
    <li>Alternatively, paste a full <code>wsh(sortedmulti(…))</code> descriptor to fill every card at once.</li>
    <li>Watch the warnings panel. Corvin flags duplicate keys, mismatched derivation paths, and malformed fingerprints, so you catch a wrong cosigner before saving.</li>
    <li>Sanity-check the descriptor preview, then click <strong>Add wallet</strong>.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the multisig wallet appears in the sidebar. To spend later, Corvin builds a PSBT that each signer signs in turn, then combines the signatures before broadcasting. Back up the wallet descriptor too (Tools, then Export multisig config); recovering needs it along with a threshold of seeds.</p>
</section>

<section class="article" data-id="hardware" hidden={active !== 'hardware'}>
  <h1>Sign with a hardware wallet</h1>
  <p>A hardware wallet keeps your keys on a dedicated device that never hands them to the computer. Corvin builds the transaction, the device shows it on its own screen, and you approve it there. The keys never touch Corvin or the network. Over USB it works with BitBox, Ledger, and Trezor. Air-gapped over QR it works with Coldcard, Blockstream Jade, Keystone, SeedSigner, Passport, and other BBQr or UR signers.</p>
  <div class="callout"><span class="callout-label">Before you start</span> You need your device, and a wallet created with that device so Corvin already knows its keys.</div>
  <ol>
    <li>When creating the wallet, use the <strong>Hardware wallet</strong> tab, or import the device's file or descriptor, so Corvin captures its fingerprint and derivation path. Without those, signing won't work.</li>
    <li>To spend, fill in the <strong>Send</strong> screen as usual.</li>
    <li>In the footer, click <strong>Hardware wallet</strong>. Connect and unlock the device when prompted, and enter any pairing code shown.</li>
    <li>Confirm the amount and destination on the device's own screen, then approve.</li>
    <li>For an air-gapped device, click <strong>QR code</strong> instead. Corvin shows the transaction as an animated code, the device signs offline, and you scan the result back. No camera? Use <strong>Import the signed PSBT</strong> in that same screen to load the signed file or paste it.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the signed transaction is broadcast. Always verify the amount and address on the device, not just in Corvin, because the device is the thing you're trusting. The first time you use a multisig wallet, the device may ask you to register its policy; check the cosigners before accepting.</p>
</section>

<!-- ───────────────────────── Privacy ───────────────────────── -->
<section class="article" data-id="spending-privately" hidden={active !== 'spending-privately'}>
  <h1>Spending privately</h1>
  <p>Bitcoin is public. Every transaction is visible forever, and anyone can try to follow the trail. Corvin can't make that go away, but it gives you several tools that, used together, make your coins much harder to track. Here's what's available and when to reach for it.</p>

  <h2>Don't reuse addresses</h2>
  <p>Reusing one address gathers every payment to it into a single public pile that's trivial to watch. Corvin hands you a fresh address for each request to avoid this. For an address you can publish and reuse that still keeps incoming payments unlinked, use {@render xref('sp-setup', 'silent payments')}. There's more on the why in {@render xref('addresses', 'Addresses')}.</p>

  <h2>Control which coins you spend</h2>
  <p>When you send, the coins you combine as inputs become publicly tied together. {@render xref('coin-control', 'Coin control')} lets you pick exactly which coins go into a transaction, so you can keep unrelated coins apart, and {@render xref('labels', 'labels')} help you track which is which. Corvin also raises {@render xref('warnings', 'privacy warnings')} when a spend would reveal more than you might intend, such as reusing an address or merging differently-labelled coins.</p>

  <h2>Break the trail with payjoin</h2>
  <p>{@render xref('payjoin-send', 'Payjoin')} is a collaborative payment where the receiver also contributes an input. The result doesn't look like an ordinary payment to chain watchers, which breaks the common assumptions used to trace funds. It needs a receiver who supports it.</p>

  <h2>Keep your watching private too</h2>
  <p>Everything above is on-chain privacy. Your network privacy is separate, and it comes down to the server you use: a public server can see which addresses you ask about. Running your own is the fix. See {@render xref('connect-backend', 'Connect a backend')}.</p>
</section>

<section class="article" data-id="payjoin-send" hidden={active !== 'payjoin-send'}>
  <h1>Send a payjoin</h1>
  <p>A payjoin is a more private way to pay someone who supports it. You start from a special invoice they give you, and Corvin coordinates the rest in the background. The idea behind it is in {@render xref('payjoin', 'How payjoin works')}.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Payjoin has to be turned on in backend settings (it's off by default). You need a software single-signature wallet, and a payjoin invoice from the receiver, which is a payment link carrying a <code>pj=</code> endpoint.</div>
  <ol>
    <li>Open the wallet and click <strong>Send</strong>.</li>
    <li>Paste the receiver's full payjoin invoice into the recipient field. Corvin recognises the <code>pj=</code> endpoint and prepares a payjoin.</li>
    <li>Set the amount if the invoice didn't fix one, and pick a fee rate as usual.</li>
    <li>Sign the original transaction when prompted. That signed transaction doubles as the fallback, so your payment is safe even if coordination fails.</li>
    <li>Corvin posts it and negotiates with the receiver. When their proposal comes back, review it and click <strong>Confirm &amp; send payjoin</strong> to re-sign and broadcast.</li>
    <li>If you'd rather not wait, click <strong>Send original instead</strong> to broadcast the plain transaction you already signed.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the payjoin is broadcast and shows in history. If the receiver doesn't answer within the fallback time, Corvin sends the original automatically, so the payment still goes through, just without the privacy gain. Only newer (v2) invoices can be coordinated; an older one should be sent normally.</p>
</section>

<section class="article" data-id="payjoin-receive" hidden={active !== 'payjoin-receive'}>
  <h1>Receive a payjoin</h1>
  <p>You can also be the receiver in a payjoin, contributing one of your own coins to the payer's transaction so both sides gain privacy.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Payjoin must be enabled in backend settings, and this flow needs a wallet connected to your own Bitcoin node.</div>
  <ol>
    <li>Open the wallet and click <strong>Receive</strong>.</li>
    <li>Start a payjoin receive. Corvin provisions a session and shows a <code>pj=</code> invoice as text and a QR code.</li>
    <li>Send that invoice to the payer. Corvin waits for them to pay it.</li>
    <li>When the payer pays, Corvin moves to a confirm step. Re-supply your recovery phrase so Corvin can sign the input it contributed.</li>
    <li>Click <strong>Confirm</strong>. Corvin posts the signed proposal back to the payer.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the collaborative transaction goes out and the payment lands in your wallet. You can cancel a waiting session anytime before it's paid.</p>
</section>

<section class="article" data-id="warnings" hidden={active !== 'warnings'}>
  <h1>Privacy warnings</h1>
  <p>When you're about to send, Corvin checks the transaction for things that would quietly leak information and flags them. A warning isn't a block. Sometimes the trade-off is fine and you send anyway. It's just making sure the leak is your choice rather than an accident.</p>
  <ul>
    <li><strong>Address reuse</strong> means you're paying an address you've paid before, which links the two payments for anyone watching.</li>
    <li><strong>Mixed labels</strong> means the send pulls together coins you'd labelled as coming from different places, tying those sources to one owner on-chain.</li>
    <li><strong>Mixed categories</strong> is the same idea for {@render xref('labels', 'categories')}: the send spends coins from two different categories together, joining compartments you'd meant to keep apart.</li>
    <li>A <strong>round-number amount</strong>, like exactly 0.1 BTC, stands out and often reveals which output is the real payment and which is your change.</li>
    <li><strong>Change address type</strong> warns when your change output is a different address type than the recipient (say, your change is <code>bc1q</code> but you're paying a <code>bc1p</code> address). An observer can then tell which output is change, since it's the one matching your own inputs. This one follows your wallet's type, so it's informational rather than something you can fix per-payment.</li>
    <li><strong>Look-alike address</strong> warns that a recipient address closely matches one already in your history, sharing its first and last characters. That's the signature of an address-poisoning scam, where an attacker seeds your history with a similar-looking address hoping you'll copy the wrong one. Check the full address, not just the ends.</li>
  </ul>
  <p>None of these are bugs in your transaction. They're hints about what it reveals. Read them, decide, and send.</p>
  <p>Separately, the verify screen shows <strong>sanity checks</strong> that aren't about privacy but about mistakes: a fee that's large relative to what you're sending, or a payment that spends nearly your whole balance. They're there to catch a fat-fingered amount or an unexpected fee before it's irreversible.</p>
</section>

<section class="article" data-id="encryption" hidden={active !== 'encryption'}>
  <h1>Encrypt your data at rest</h1>
  <p>Corvin can encrypt everything it stores on disk (wallet data, labels, notes, categories, and settings) with a password of your choosing. It's optional and off by default. A fresh install runs unencrypted, exactly as before, until you turn this on.</p>
  <h2>What it protects, and what it doesn't</h2>
  <p>This is a privacy feature, not theft protection for your coins. Corvin never writes your {@render xref('seed', 'recovery phrase')} or any spending key to disk, so a stolen drive can already never spend your funds, encrypted or not. What a copied or seized config folder <em>can</em> read in plain form is your wallet's public keys (which reveal your whole balance and transaction history going forward), your Silent Payments scan key, and all your labels and notes. Encryption is what keeps those private if the folder ever leaves your control, whether through a stolen device, a backup, or a synced directory.</p>
  <div class="callout"><span class="callout-label">Good to know</span> On a normal desktop with full-disk encryption (LUKS, FileVault, BitLocker) already turned on, that covers the stolen-machine case for you. This feature matters most for backups, copied config folders, and self-hosted setups like Start9 where the disk isn't otherwise encrypted.</div>
  <h2>Turn it on</h2>
  <ol>
    <li>In the sidebar, open <strong>Security</strong> (the first run's setup wizard also offers it as a "Protect your data" step).</li>
    <li>Choose a password (at least 8 characters) and confirm it.</li>
    <li>Click <strong>Encrypt wallet data</strong>. Corvin rewrites everything it has stored into encrypted form in place.</li>
  </ol>
  <p>The password is turned into a key with Argon2id, a deliberately slow function that makes guessing expensive, and that key never touches the disk. Only you hold it.</p>
  <h2>What changes after that</h2>
  <p>When encryption is on, Corvin boots <strong>locked</strong>. After a restart it loads nothing and shows an unlock screen first; you enter your password and the app decrypts into memory and starts up. The trade-off worth knowing: while locked, background sync and new-payment notifications are paused, so after a reboot they only resume once someone unlocks. On a desktop that's the moment you open the app. On a server it's whenever you next visit the page.</p>
  <div class="callout danger"><span class="callout-label">No recovery</span> The password is never stored, so if you forget it there is no way to recover the encrypted data. You would lose your labels, notes, and settings, never your money: re-add each wallet from its recovery phrase or xpub and the coins and history come straight back. Pick a password you won't lose, and keep your seed backup as solid as ever.</div>
  <h2>Backups are separate</h2>
  <p>This protects the copy of your data <em>on this device</em>. A backup you export from {@render xref('import-export', 'Import and export data')} is its own file and is plain JSON unless you set a passphrase on it. So if you turn this on, set a passphrase when you export a backup too, otherwise that file is unprotected even though the device copy is encrypted.</p>
  <h2>Change your password</h2>
  <p>On the <strong>Security</strong> page, the <strong>Change password</strong> card asks for your current password and a new one. Corvin re-encrypts everything under the new key in a single pass, so your data is never written to disk in plain form during the change. If you only suspect your password was seen, this is faster and safer than disabling and re-enabling.</p>
  <h2>Turn it off</h2>
  <p>Open <strong>Security</strong> and choose <strong>Disable encryption</strong>. Because this rewrites everything to plain form on disk, Corvin asks for your password to confirm before it does. Once disabled, it stops booting locked. You can re-enable it later with a new password whenever you like.</p>
</section>

<!-- ───────────────────────── Backups & records ───────────────────────── -->
<section class="article" data-id="backup" hidden={active !== 'backup'}>
  <h1>Back up and restore</h1>
  <p>Two different things are worth backing up, and they aren't the same.</p>
  <h2>The keys</h2>
  <p>Your {@render xref('seed', 'recovery phrase')}, along with any hardware-wallet backups, is what restores the money. With the phrase alone you can rebuild the wallet from scratch on any compatible software. That's the floor, and it isn't optional.</p>
  <h2>The context</h2>
  <p>The phrase doesn't carry your labels, your notes, your frozen coins, or your wallet setup. Corvin's full backup exports all of that, so a restore brings back a wallet that looks like the one you had rather than a blank one with the same coins. Labels also export on their own in the BIP-329 format, which is a shared standard other wallets understand, handy if you ever move.</p>
  <p>Restoring is the reverse. Import the backup, then re-supply your phrase when it's time to sign. It's worth doing a dry run once, on a wallet that matters, so you know the process before you need it in a hurry.</p>
</section>

<section class="article" data-id="labels" hidden={active !== 'labels'}>
  <h1>Notes and categories</h1>
  <p>Two kinds of private annotation make your history readable later. Both stay on your machine, never go on-chain, and travel with your backup.</p>
  <h2>Notes</h2>
  <p>A <strong>note</strong> is free text you attach to one thing: a transaction, an address, or a coin. Use it to remember what something was, like "rent" or "from the exchange".</p>
  <ol>
    <li>To note a transaction, open it from the history list and type in its note field.</li>
    <li>To note a receive address, open the wallet's addresses list and type in the <strong>Note</strong> column on the row.</li>
    <li>To note a coin, do the same in the <strong>Note</strong> column on the coins (UTXO) list.</li>
  </ol>
  <h2>Categories</h2>
  <p>A <strong>category</strong> is a reusable, coloured tag you define once and apply to many addresses and coins, like "Savings", "Spending", or "KYC". Where a note is a one-off reminder, a category groups coins into compartments you want to keep apart. You set a category on an <strong>address</strong>, and the coins received there inherit it; you can also override a single coin's category on the coins list.</p>
  <ol>
    <li>On the addresses or coins list, use the category picker on the row to assign a category, or create a new one with a name and colour.</li>
    <li>A coloured chip then marks every address and coin in that category, so its colour is easy to spot in a list.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> your notes and category chips show up wherever that transaction, address, or coin appears. Categories also feed coin control and the warning that flags spending coins from <em>different</em> categories together, alongside the older mixed-labels check (see {@render xref('warnings', 'Privacy warnings')}). Notes move between wallets with the BIP-329 export, covered in {@render xref('import-export', 'Import and export data')}.</p>
</section>

<section class="article" data-id="tax" hidden={active !== 'tax'}>
  <h1>Export a tax report</h1>
  <p>Corvin can produce a gain and loss report for a year from your transaction history and stored price data.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Turn on historical price data in Display settings, otherwise cost basis can't be worked out.</div>
  <ol>
    <li>Open the wallet's <strong>Tools</strong> tab and choose <strong>Tax report</strong>.</li>
    <li>Pick the <strong>Year</strong>.</li>
    <li>Pick a <strong>Method</strong>: HIFO (highest cost first), FIFO (oldest first), or LIFO (newest first).</li>
    <li>Review the net gain and loss summary and the per-disposal rows.</li>
    <li>Click <strong>Download CSV</strong> to save the report.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> a CSV named for the wallet, year, and method, ready for a spreadsheet or an accountant. This is a convenience report, not tax advice.</p>
</section>

<section class="article" data-id="import-export" hidden={active !== 'import-export'}>
  <h1>Import and export data</h1>
  <p>Two kinds of export live on the Import / Export page: a full Corvin backup, and your labels in the shared BIP-329 format.</p>
  <ol>
    <li>Open <strong>Import / Export</strong> from the sidebar.</li>
    <li>For a full backup, click <strong>Export</strong> to download your wallets, labels, and settings. If any wallet holds silent-payments keys, Corvin requires a passphrase and encrypts the file before download.</li>
    <li>To restore, choose your backup file. Corvin shows a summary of what's inside before you apply it.</li>
    <li>For labels only, use the BIP-329 export to save them as a portable file, or import one from another wallet, optionally replacing existing labels.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> your data is saved or restored. Remember that a full backup is context, not keys. Your recovery phrase is still what restores the actual money, covered in {@render xref('backup', 'Back up and restore')}.</p>
</section>

<!-- ───────────────────────── Tools ───────────────────────── -->
<section class="article" data-id="broadcast" hidden={active !== 'broadcast'}>
  <h1>Broadcast a transaction</h1>
  <p>If you have a signed transaction from somewhere else, such as a hardware wallet, another app, or a cosigner, Corvin can push it to the network for you.</p>
  <ol>
    <li>Open <strong>Broadcast…</strong> from the wallet's <strong>⋯ menu</strong>.</li>
    <li>Paste a signed PSBT (base64) or a raw transaction (hex), or load a <code>.psbt</code> file.</li>
    <li>Decode it, then review what it spends and where it sends.</li>
    <li>If it looks right, click <strong>Broadcast</strong>.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> Corvin sends the transaction and shows its txid. It appears in the relevant wallet's history once the network sees it. To look a transaction over without sending, use {@render xref('psbt-inspector', 'the PSBT inspector')} instead.</p>
</section>

<section class="article" data-id="psbt-inspector" hidden={active !== 'psbt-inspector'}>
  <h1>Inspect a PSBT</h1>
  <p>The PSBT inspector decodes any partially signed transaction so you can see exactly what it does. It never signs and never broadcasts, so it's safe to paste anything in.</p>
  <ol>
    <li>Open <strong>PSBT inspector</strong> from the wallet's <strong>Tools</strong> tab.</li>
    <li>Paste any PSBT.</li>
    <li>Read the summary: how much leaves the wallet, how much returns to you as change, and the fee with its rate.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> a clear picture of the transaction with nothing committed. It's the right way to sanity-check a PSBT a cosigner or an external app handed you before you sign it.</p>
</section>

<section class="article" data-id="consolidate" hidden={active !== 'consolidate'}>
  <h1>Consolidate UTXOs</h1>
  <p>Consolidating combines many small coins into one. That makes future sends cheaper, because a payment then spends fewer inputs. The trade-off is privacy: pulling those coins together publicly links them to the same owner, so it's a deliberate choice rather than routine housekeeping.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Pick a quiet, low-fee moment. Since consolidating is itself a transaction, the cheapest time to do it is when network fees are low.</div>
  <ol>
    <li>Open <strong>Consolidate UTXOs</strong> from the wallet's <strong>⋯ menu</strong>.</li>
    <li>Pick a fee rate. Corvin warns if your custom rate is well below current network rates.</li>
    <li>Choose the destination: a fresh address in this wallet, or another of your wallets.</li>
    <li>Review and sign the transaction the usual way, then broadcast.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> your many coins become one or a few, so later payments need fewer inputs and cost less. The reason input count drives cost is in {@render xref('fees', 'Fees and sat/vB')}.</p>
</section>

<section class="article" data-id="messages" hidden={active !== 'messages'}>
  <h1>Sign or verify a message</h1>
  <p>Signing a message proves you control an address without spending from it, which is handy for proving ownership to an exchange or a counterparty. Verifying checks someone else's signed message. Corvin uses the BIP-322 standard.</p>
  <div class="callout"><span class="callout-label">Before you start</span> Signing needs a software HD wallet. Multisig and watch-only address wallets can't produce a single-key message signature.</div>
  <ol>
    <li>Open <strong>Messages</strong> from the wallet's <strong>Tools</strong> tab, and choose <strong>Sign</strong> or <strong>Verify</strong>.</li>
    <li>To sign: pick one of your addresses, type the message, and supply your recovery phrase. Corvin returns a signature you can share.</li>
    <li>To verify: paste the address, the message, and the signature. Corvin tells you whether they match.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> a signature that proves you hold an address, or a clear pass or fail on one you were given.</p>
</section>

<section class="article" data-id="address-lookup" hidden={active !== 'address-lookup'}>
  <h1>Look up an address</h1>
  <p>Address lookup tells you whether an address belongs to this wallet and where it sits in the derivation tree. It's a quick check that runs entirely on your machine.</p>
  <ol>
    <li>Open <strong>Address lookup</strong> from the wallet's <strong>Tools</strong> tab.</li>
    <li>Paste an address.</li>
    <li>Corvin shows whether it matches this wallet, and if so its script type and derivation path. For multisig it shows each cosigner's branch.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> confirmation that an address is yours, or that it isn't. Useful before sharing an address, or when reconciling your own records.</p>
</section>

<section class="article" data-id="export-keys" hidden={active !== 'export-keys'}>
  <h1>Export keys and descriptors</h1>
  <p>A wallet's public details can be exported so other software can watch it, or so you can record it for recovery. None of these files contain spending keys, so they can't move funds on their own.</p>
  <ol>
    <li>Open the wallet's <strong>Tools</strong> tab.</li>
    <li><strong>Export descriptor</strong> saves the wallet's output descriptor, which any compatible wallet can import as watch-only.</li>
    <li>For a silent-payments wallet, <strong>Export watch-only keys</strong> saves the scan key and spend pubkey, so someone can monitor incoming payments without being able to spend.</li>
    <li>For a multisig wallet, <strong>Export multisig config</strong> saves the coordinator file (Coldcard-compatible) used to set up each signer's device.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> a portable file describing the wallet's public side. Keep these with your records. They help recovery, but the actual money still needs your {@render xref('seed', 'recovery phrase')} to move.</p>
</section>

<section class="article" data-id="sweep" hidden={active !== 'sweep'}>
  <h1>Sweep a private key</h1>
  <p>Sweeping moves every coin controlled by a single private key into one of your wallets. It's how you empty a paper wallet or pull funds out of a key from another app.</p>
  <div class="callout"><span class="callout-label">Before you start</span> You need the private key in WIF format, and a wallet to sweep the funds into.</div>
  <ol>
    <li>Open the wallet's <strong>Tools</strong> tab and choose <strong>Sweep private key</strong>.</li>
    <li>Paste the private key in WIF format. The key is held in memory only and wiped after signing.</li>
    <li>Set the destination. Corvin fills in a fresh receive address from the current wallet by default.</li>
    <li>Corvin scans the key's addresses across legacy, SegWit, and wrapped-SegWit types and shows a preview of what it found.</li>
    <li>Broadcast the sweep to move the funds.</li>
  </ol>
  <p class="outcome"><strong>Result:</strong> the coins land in your destination wallet. The original key is now empty, so you can import or destroy it as you see fit.</p>
</section>

<!-- ───────────────────────── Troubleshooting ───────────────────────── -->
<section class="article" data-id="troubleshooting" hidden={active !== 'troubleshooting'}>
  <h1>Troubleshooting</h1>
  <p>Common problems and what to check first. When something fails, a wallet's connection status (on its page, and the dot beside it in the sidebar) and the message on the failed action are usually the best clues.</p>

  <h2>Can't connect to the backend</h2>
  <p>The wallet's page shows its server and connection state, and the sidebar shows a red dot next to a wallet whose server is unreachable. Check the host and port first. If the server uses TLS, make sure <strong>SSL</strong> is on (public Electrum servers usually listen on port 50002 for SSL). For a <code>.onion</code> server, turn on the SOCKS5 proxy and point it at your Tor proxy, typically <code>127.0.0.1:9050</code>. Use <strong>Test connection</strong> when adding or editing the server to confirm before saving.</p>
  <p>While a backend is unreachable, the wallet shows an amber banner and keeps working in a degraded state: it displays your <strong>last-synced</strong> data, and you can still build, sign, and export transactions — only syncing and broadcasting wait for the connection to return. Corvin reconnects on its own, or you can hit <strong>Retry</strong>. (This is different from {@render xref('offline', 'offline mode')}, which you turn on deliberately.)</p>

  <h2>A self-signed certificate is rejected</h2>
  <p>If your own Electrum server uses a self-signed certificate, edit that server under Backend → <strong>Saved backends</strong> and, in its Advanced options, either tick <strong>Accept invalid / self-signed certificates</strong> or set the <strong>CA certificate path</strong> to its PEM or DER file. Only do this for a server you control.</p>

  <h2>My hardware wallet isn't detected</h2>
  <p>Make sure the device is unlocked, and for Ledger that the Bitcoin app is open and Ledger Live is closed (only one program can talk to it at a time). On Linux the system needs udev rules to allow access without root. Corvin ships them at <code>packaging/udev/51-corvin.rules</code>: install them, then unplug and replug the device.</p>

  <h2>My addresses or balance look wrong</h2>
  <p>If a restored wallet shows different addresses than you expected, the usual cause is a BIP39 passphrase. A different passphrase, or a typo in one, produces a completely different wallet from the same words (an empty passphrase is itself a specific choice). Re-add the wallet with the exact passphrase you used originally. See {@render xref('seed', 'Recovery phrases')}.</p>

  <h2>Funds are missing after restoring</h2>
  <p>A freshly added wallet scans your address history on first sync and extends the window as it finds activity, so give it a moment and check again. Corvin scans a fixed number of unused addresses ahead (a gap of twenty). If you previously skipped well past that many unused addresses, those coins may not appear on their own; receiving into the wallet normally, or re-adding it, lets the scan catch up.</p>

  <h2>My wallets disappeared</h2>
  <p>Corvin runs one network at a time. If you switched the network in Backend settings, wallets from the other network are hidden until you switch back. They're still in storage and reappear when you return to their network. A network switch fully takes effect after a restart.</p>

  <h2>Does showing the fiat price leak my balance?</h2>
  <p>No. Price data is optional and only fetched when you set a mempool URL. Corvin asks that server for the Bitcoin price on a given date, then multiplies it by your balance locally. Your addresses, balances, and transactions are never sent, and the request honors the SOCKS5 proxy if you've set one.</p>

  <h2>Where does Corvin keep my data?</h2>
  <p>Everything lives under <code>~/.config/corvin/</code>: your settings, the wallet list, and the per-wallet databases. Your recovery phrase is never written there. Backing up that folder preserves your labels and settings, but it is not a substitute for backing up your seed. See {@render xref('backup', 'Back up and restore')}.</p>

  <h2>Reporting a problem</h2>
  <p>Open <strong>Backend settings</strong> and use <strong>Copy debug info</strong> under <strong>Diagnostics</strong> to put a short report on your clipboard: the app version, your platform, connection status, and a count of wallets by kind. It deliberately contains no seeds, keys, addresses, labels, or balances, so it's safe to paste into a bug report. Nothing is sent anywhere on its own; you choose where it goes.</p>
</section>

<!-- ───────────────────────── Concepts ───────────────────────── -->
<section class="article" data-id="wallet-kinds" hidden={active !== 'wallet-kinds'}>
  <h1>Wallet kinds explained</h1>
  <p>When you add a wallet, Corvin asks which kind you want. They behave differently enough that it's worth knowing the difference up front, because you can't convert one into another later.</p>
  <h2>Single signature</h2>
  <p>The ordinary kind. One recovery phrase controls the money, and one signature spends it. It's quick to set up, easy to back up, and the right choice for everyday amounts. You can generate a fresh phrase or import one you already have.</p>
  <h2>Multisig</h2>
  <p>Spending needs several signatures out of a group, and a 2-of-3 setup is common. No single phrase or device can move the funds alone, which is what you want for long-term savings or anything you'd hate to lose to one stolen laptop. There's more to manage, since every signer needs its own backup and you'll want all of them recorded.</p>
  <h2>Silent payments</h2>
  <p>A wallet built around one reusable address you can publish anywhere without linking your payments together. It works differently under the hood, so it gets its own topic. See {@render xref('sp-concept', 'How silent payments work')} for the detail worth understanding before you rely on it.</p>
  <h2>Watch-only</h2>
  <p>Tracks an address or an extended public key but holds no spending keys at all. It can show you balances and history, but it can't sign anything. It's useful for keeping an eye on cold storage, or on funds that live on a hardware wallet you sign with separately.</p>
  <h2>Vault / Policy</h2>
  <p>Timelock templates for funds you want to lock up. An inheritance vault hands control to a recovery group after a set delay, and a savings policy locks coins until a future date. These are advanced setups, so reach for them once the simpler kinds feel familiar.</p>
</section>

<section class="article" data-id="addresses" hidden={active !== 'addresses'}>
  <h1>Addresses</h1>
  <p>An address is where someone sends coins to you. Corvin hands out a fresh one each time on purpose. Reusing an address still works, but it lets anyone looking at the blockchain tie those payments together and watch the balance behind them. A new address per request avoids that.</p>
  <p>All of those addresses belong to the same wallet, so you don't have to track them. Old ones keep working forever, which means a printed invoice or a saved address won't break.</p>
  <p>The Receive screen shows the next address as text and as a QR code. If you want a clean separation between, say, an exchange withdrawal and a private payment, a label on the address and on the resulting coins keeps them straight later. See {@render xref('coin-control', 'Use coin control')}.</p>
</section>

<section class="article" data-id="seed" hidden={active !== 'seed'}>
  <h1>Recovery phrases</h1>
  <p>The recovery phrase is twelve or twenty-four words, also called a seed or a mnemonic, and it is your wallet. Every address and key comes from it. Whoever has the words has the money, and whoever loses them loses the money. Corvin never stores the phrase on disk. It asks for it only at the moment it needs to sign something, then wipes it from memory.</p>
  <p>That puts the backup squarely on you. A few rules that have saved a lot of people:</p>
  <ul>
    <li>Write it on paper or steel, by hand. Don't photograph it, don't paste it into notes, and don't email it to yourself.</li>
    <li>The order of the words matters. Number them.</li>
    <li>Store at least one copy somewhere a house fire or a burglar won't reach along with your computer.</li>
    <li>Anyone who asks for your phrase is trying to rob you. Corvin will never ask for it to verify or upgrade anything. It only asks in order to sign a specific action you started.</li>
  </ul>
  <h2>Passphrase</h2>
  <p>An optional extra word of your own, sometimes called a 13th or 25th word. It changes the wallet completely, because the same phrase with a different passphrase is a different wallet. It adds protection if your written phrase is ever found, but it's also one more thing to lose. Forget the passphrase and the funds are gone even with the words in hand. Treat it as seriously as the phrase itself.</p>
</section>

<section class="article" data-id="sp-concept" hidden={active !== 'sp-concept'}>
  <h1>How silent payments work</h1>
  <p>A silent payment address is one reusable address that starts with <code>sp1</code>. You can post it publicly, hand it out repeatedly, or print it on a tip jar, and the payments to it never appear linked on the blockchain. Each payer derives a unique, unguessable address for you from yours, so on-chain it looks like unrelated coins landing in unrelated places. You get the convenience of a static address with the privacy of fresh ones.</p>
  <p>Finding those payments takes more work than a normal wallet does. There's no fixed address to watch, so Corvin scans transactions for the ones meant for you using your <strong>scan key</strong>. It runs this in the background against a Frigate (BIP-352) server, chosen when you create the wallet — a public one, or your own if you've saved a Frigate backend.</p>
  <h2>The detail worth knowing</h2>
  <p>To scan for you, that server is given your scan key. While it's connected, it can see which payments are yours to receive. That isn't enough to spend anything, and it never touches your spending key, but it does learn what's coming in. If that matters to you, run the scanner against your own server. This is a real trade-off in how silent payments work rather than a Corvin quirk, and it's why the kind is kept separate from your ordinary wallets.</p>
  <h2>Spending</h2>
  <p>Coins received this way spend like any others from inside the silent-payments wallet. Behind the scenes each one is unlocked with a key rebuilt from your recovery phrase at signing time.</p>
</section>

<section class="article" data-id="payjoin" hidden={active !== 'payjoin'}>
  <h1>How payjoin works</h1>
  <p>Payjoin is a way for a sender and receiver to build a payment together, where the receiver quietly adds one of their own coins to the transaction. The result still pays the right amount, but it breaks a common assumption that blockchain surveillance relies on, namely that every input to a transaction belongs to the sender. To an outside observer the payment becomes hard to read, and the receiver's coins get a privacy boost too.</p>
  <p>It happens automatically once both sides support it. You approve the payment as usual and the coordination occurs behind the scenes. If the other side doesn't answer in time, Corvin falls back to sending the ordinary transaction it already prepared, so your payment goes through either way, just without the privacy bonus.</p>
  <p>Payjoin is off by default, and you enable it in backend settings. Sending works from software single-signature wallets. Receiving needs a wallet connected to your own Bitcoin node.</p>
</section>

<section class="article" data-id="sending-concept" hidden={active !== 'sending-concept'}>
  <h1>How sending works</h1>
  <p>When you send, Corvin first builds an unsigned <strong>PSBT</strong>, short for partially signed Bitcoin transaction. That's just the standard container wallets and signing devices pass around. Nothing is on the network yet, and you can throw it away at no cost. Only after something signs it does Corvin broadcast it.</p>
  <p>What signs it depends on the wallet. A software wallet signs with keys rebuilt from your recovery phrase. A hardware wallet shows the details on its own screen for you to approve, covered in {@render xref('hardware', 'Sign with a hardware wallet')}.</p>
  <h2>Change</h2>
  <p>Bitcoin spends whole coins. If you send 0.2 from a 0.5 coin, the leftover 0.3 comes back to you as <strong>change</strong>, sent to a fresh address in your own wallet. It isn't lost and it isn't a fee. It's just how the arithmetic works, and your balance reflects it right away. There's more on this in {@render xref('coins-concept', 'Coins, UTXOs, and change')}.</p>
</section>

<section class="article" data-id="fees" hidden={active !== 'fees'}>
  <h1>Fees and sat/vB</h1>
  <p>A fee isn't a fixed price. You're bidding for space in an upcoming block, and miners take the highest bids first. The bid is measured in <strong>satoshis per virtual byte</strong>, written sat/vB. It's the price per unit of transaction size, not per amount sent.</p>
  <p>That last part catches people out. A transaction's size depends mostly on how many coins it pulls together, not on how much it moves. Sending a tiny amount cobbled from many small coins can cost more than sending a fortune from one big coin. {@render xref('coin-control', 'Coin control')} is how you keep an eye on that.</p>
  <p>The wallet's status line shows current rates from your mempool server, usually as slow, medium, and fast. Pick higher when you're in a hurry and lower when you can wait. If you guess too low and it sits unconfirmed, you're not stuck. See {@render xref('speed-up', 'Speed up a stuck payment')}.</p>
</section>

<section class="article" data-id="coins-concept" hidden={active !== 'coins-concept'}>
  <h1>Coins, UTXOs, and change</h1>
  <p>Your balance is really a pile of separate coins, called <strong>UTXOs</strong>, short for unspent transaction outputs. Each one is a payment you received that hasn't been spent yet. When you send, Corvin gathers up enough coins to cover the amount plus the fee, and whatever's left over comes back as change.</p>
  <p>This is why a wallet showing 1 BTC isn't holding a single 1 BTC object. It might be ten coins of different sizes from ten different payments. Spending always consumes whole coins, so the wallet picks a set that covers what you need.</p>
  <p>Two things follow from that. Fees depend on how many coins you pull together, not how much you move, which is covered in {@render xref('fees', 'Fees and sat/vB')}. And spending several coins at once publicly ties them to the same owner, which is where {@render xref('coin-control', 'coin control')} earns its keep.</p>
</section>

<section class="article" data-id="glossary" hidden={active !== 'glossary'}>
  <h1>Glossary</h1>
  <p>Short definitions for the words that show up in the interface.</p>
  <dl class="glossary">
    <dt>Address</dt>
    <dd>A destination you give someone so they can pay you. Corvin uses a fresh one each time.</dd>
    <dt>Backend</dt>
    <dd>The server Corvin connects to for chain data: a public or private Electrum server, or your own Bitcoin node (Bitcoin Core, Knots, etc.).</dd>
    <dt>BIP-329</dt>
    <dd>A shared file format for wallet labels, so notes can move between wallets.</dd>
    <dt>Change</dt>
    <dd>The leftover that returns to your own wallet when a coin is bigger than the amount you're sending.</dd>
    <dt>CPFP</dt>
    <dd>Child-pays-for-parent. Confirm a stuck incoming payment by spending it with a high fee.</dd>
    <dt>Descriptor, or xpub</dt>
    <dd>A compact description of all of a wallet's addresses, derived from an extended public key. It reveals balances, not the ability to spend.</dd>
    <dt>Multisig</dt>
    <dd>A wallet that needs several signatures out of a group to spend, such as 2-of-3.</dd>
    <dt>PSBT</dt>
    <dd>Partially Signed Bitcoin Transaction, the standard container an unsigned transaction travels in before it's signed.</dd>
    <dt>Payjoin</dt>
    <dd>A collaborative payment where the receiver adds an input, improving privacy for both sides.</dd>
    <dt>RBF</dt>
    <dd>Replace-by-fee. Resend your own unconfirmed transaction with a higher fee to speed it up.</dd>
    <dt>Recovery phrase</dt>
    <dd>The 12 or 24 words that are your wallet. Back them up, and never share them.</dd>
    <dt>sat/vB</dt>
    <dd>Satoshis per virtual byte, the fee rate. It's the price you pay per unit of transaction size.</dd>
    <dt>Satoshi</dt>
    <dd>The smallest unit of bitcoin. 100,000,000 sats make 1 BTC.</dd>
    <dt>Silent payment</dt>
    <dd>A reusable <code>sp1</code> address that stays private, because payments to it don't link on-chain.</dd>
    <dt>UTXO</dt>
    <dd>An unspent coin, one received payment you haven't spent yet. Your balance is a pile of these.</dd>
    <dt>Vault</dt>
    <dd>A wallet with a timelock policy, such as funds locked until a date or handed to a recovery group after a delay.</dd>
    <dt>Watch-only</dt>
    <dd>A wallet that can see funds but holds no keys, so it can't sign or spend.</dd>
  </dl>
</section>

<style>
  h1 {
    font-size: 1.5rem;
    font-weight: 700;
    letter-spacing: -0.01em;
    margin: 0 0 18px;
    color: var(--text);
  }
  h2 {
    font-size: 1.02rem;
    font-weight: 600;
    margin: 28px 0 8px;
    color: var(--text);
  }
  p {
    font-size: 0.92rem;
    line-height: 1.65;
    color: var(--text-muted);
    margin: 0 0 14px;
  }
  ul { margin: 0 0 14px; padding-left: 20px; }
  ol { margin: 0 0 16px; padding-left: 22px; }
  li {
    font-size: 0.92rem;
    line-height: 1.6;
    color: var(--text-muted);
    margin-bottom: 9px;
    padding-left: 3px;
  }
  strong { color: var(--text); font-weight: 600; }
  code {
    font-family: var(--font-mono);
    font-size: 0.85em;
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 1px 5px;
    color: var(--text);
  }
  a { color: var(--accent); text-decoration: none; }
  a:hover { text-decoration: underline; }

  .callout {
    background: var(--surface-1);
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: var(--radius-sm);
    padding: 11px 14px;
    margin: 0 0 18px;
    font-size: 0.88rem;
    line-height: 1.55;
    color: var(--text-muted);
  }
  .callout.danger {
    border-left-color: #e09c52;
    background: color-mix(in srgb, #e09c52 8%, var(--surface-1));
  }
  .callout-label {
    display: block;
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text);
    font-weight: 600;
    margin-bottom: 3px;
  }
  .outcome {
    border-top: 1px solid var(--border);
    padding-top: 14px;
    margin-top: 18px;
  }

  .glossary { margin: 4px 0 0; }
  .glossary dt {
    font-weight: 600;
    color: var(--text);
    font-size: 0.9rem;
    margin-top: 16px;
  }
  .glossary dd {
    margin: 3px 0 0;
    font-size: 0.9rem;
    line-height: 1.55;
    color: var(--text-muted);
  }
</style>
