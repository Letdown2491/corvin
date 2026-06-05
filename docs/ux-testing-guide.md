# Corvin UX / UI testing guide

A structured guide for handing Corvin (desktop, macOS / Windows) to outside testers.
The goal is **not** to verify that features technically work; it's to find out whether
someone who *isn't the developer* finds the wallet clear, trustworthy, and pleasant to
use. Functional/security verification is a separate pass; this is about
**usability**.

Most of this is fill-in-the-blank. Testers grade tasks as they go and answer a few
questions at the end. The organizer collects the filled-in copies.

---

## Part 0: for the organizer (fill this in before sending)

Replace the bracketed bits, and delete this box from the tester's copy if you like.

- **Build:** `[ link to the signed .dmg / .msi or instructions ]`
- **Network for this test:** `[ signet recommended for send/receive (free coins, no real-money risk), or mainnet watch-only ]`
- **Test funds:** `[ how testers get coins to play with, e.g. a signet faucet link, or "I'll send you some" ]`
- **Inputs for the Tools scenario:** have ready a **sample PSBT** (base64/hex) for the PSBT inspector, and, only if you want them to try **Sweep private key**, a **WIF holding a little test-network value** they can drain. (Sign-message, address lookup, export, test backup, and tax report need no extra inputs.)
- **Where to send results:** `[ email / Signal / GitHub issue / shared doc ]`
- **Time:** plan for **30-45 min**. It's fine if they don't finish every scenario.

**How to run a session (if you're watching live):**
- Ask them to **think aloud**: narrate what they're looking at, what they expect, what confuses them. This is gold.
- **Don't help** unless they're truly stuck (give it about 60-90s). Note *where* and *why* they got stuck; that's the finding.
- Don't defend or explain the design. Just observe and take notes.
- If testing async (they run it alone), tell them to jot notes in the moment, not from memory afterward.

**Don't pre-train them.** First impressions are the whole point; if you explain how it works first, you've destroyed the data.

**Install warnings are not app UX.** If the build isn't notarized (macOS) / signed (Windows), testers will hit "*can't be opened, Apple cannot check it*" or a SmartScreen "*unrecognized app*" prompt. That's a real trust hit, but it's about your distribution, not Corvin's design, so tell testers how to get past it (and that it's expected for a test build) so it doesn't skew the trust/first-impression answers. Note separately whether the install step itself scared anyone.

---

## Part 1: for the tester, how to use this guide

You're helping make sure Corvin is good enough for normal people to use. There are no
wrong answers and **nothing here is a test of you**: if something is confusing, that's a
bug in the app, not in you. Be blunt; polite feedback is useless feedback.

**For each task below:**
1. Try to do it **without help**. Narrate your thoughts out loud if someone's watching.
2. Rate how it went on the **Ease scale**.
3. Write a quick note, especially anything that confused you, made you hesitate, or felt wrong.

**Ease scale (use this everywhere a 1-5 is asked):**

| Score | Meaning |
|---|---|
| **5** | Effortless: I knew exactly what to do, no hesitation |
| **4** | Easy: minor pause but figured it out fast |
| **3** | OK: got there, but had to stop and think / hunt around |
| **2** | Hard: confused, guessed, nearly gave up |
| **1** | Failed / gave up / needed help |

If you hit anything that feels broken or scary, log it in the **Issue log** at the end with a severity.

---

## Part 2: about you (1 minute)

This just gives your answers context; there are no right answers.

- Operating system: ☐ macOS  ☐ Windows  (version: `____`)
- How would you describe your Bitcoin experience?
  ☐ New-ish  ☐ Comfortable  ☐ Power user
- Wallets you've used before: `________________________`
- Have you ever run your own node / Electrum server? ☐ Yes ☐ No
- Do you currently self-custody (hold your own keys)? ☐ Yes ☐ No

---

## Part 3: task scenarios

Do these in order. Each has a **task**, then questions. Don't read ahead; react to what's
in front of you.

### Scenario 1: first impressions (don't click anything yet)

Open the app and look at the first screen for about 15 seconds before doing anything.

- In one sentence, what do you think this app is and what's it for? `________________`
- What's the **first thing** you feel you're supposed to do? `________________`
- Does it look trustworthy / legit for holding money? ☐ Yes ☐ Unsure ☐ No. Why? `____`
- First-impression rating (look & feel): **1 2 3 4 5**
- Notes: `________________________________________________`

### Scenario 2: create a new wallet (your first one)

Task: **Make a brand-new wallet you control.** Go as far as the app asks you to,
including anything about writing down a recovery phrase.

- Ease: **1 2 3 4 5**
- Did you understand the **recovery phrase / backup** step? Did it feel important and clear? `____`
- The step that asked you to confirm some words: was it obvious why? Annoying? Reassuring? `____`
- Did you know when the wallet was "done" and ready? ☐ Yes ☐ No
- Were any words/buttons confusing (e.g. "descriptor", "xpub", "passphrase", "account")? Which? `____`
- Notes: `________________________________________________`

### Scenario 3: receive (get an address)

Task: **Get an address someone could send Bitcoin to.** Imagine you're going to paste it
to a friend.

- Ease: **1 2 3 4 5**
- Did you feel confident the address was safe to share and yours? ☐ Yes ☐ Unsure ☐ No
- Was the QR code / copy button where you expected? ☐ Yes ☐ No
- Did anything warn you or advise you (e.g. about reusing an address)? Was it helpful or confusing? `____`
- Notes: `________________________________________________`

### Scenario 4: read your transactions

Task: **Find your transaction history and open the details of one transaction.** (If the
wallet has none yet, the organizer will point you at one that does.)

- Ease: **1 2 3 4 5**
- Could you tell, at a glance, money **in** vs money **out**, and how much? ☐ Yes ☐ No
- Was the date / amount / fee information clear? Anything you wanted that wasn't there? `____`
- Did you try to label or search a transaction? Could you? ☐ Tried, worked ☐ Tried, failed ☐ Didn't try
- Notes: `________________________________________________`

### Scenario 5: send (use test coins only)

> **Only use the test network / test coins the organizer set up. Never send real funds you can't lose.**

Task: **Send a small amount to an address the organizer gives you.** Go through to the
point where you'd normally confirm/sign; follow the prompts.

- Ease: **1 2 3 4 5**
- Choosing the **fee / speed**: did you understand the options and feel in control? `____`
- Did the app show you anything before sending that helped you double-check (amounts, address, warnings)? Helpful or noisy? `____`
- At any point were you unsure whether the money had actually been sent? `____`
- Did you ever feel nervous you'd make an irreversible mistake? Where? `____`
- Notes: `________________________________________________`

### Scenario 6: restore / import an existing wallet

Task: **Add a second wallet from a recovery phrase or an xpub** the organizer gives you (a
watch-only one is fine).

- Ease: **1 2 3 4 5**
- Was it clear which option to pick for "I already have a wallet"? ☐ Yes ☐ No
- Did the choices on the "Add wallet" screen make sense (the different wallet types)? Which, if any, were confusing? `____`
- Notes: `________________________________________________`

### Scenario 7: find your way around

Task (no clicking guidance): find each of these. Tick if you found it in under about 20s.

- ☐ Switch between your wallets
- ☐ The settings for how the app connects to the network ("backend")
- ☐ Where you'd change light/dark theme or units
- ☐ The Help / docs
- ☐ Your wallet's total balance and whether it's up to date / synced

- Overall, did you always know **where you were** and how to get back? ☐ Yes ☐ Mostly ☐ No
- Navigation rating: **1 2 3 4 5**
- Anything you went looking for and couldn't find? `____`

### Scenario 8: settings & connection

Task: **Open the Backend settings and the Display settings and change something** (e.g.
toggle a display option, look at the server settings). You don't have to understand
everything.

- Ease: **1 2 3 4 5**
- When you changed something, did you know it was **saved**? ☐ Yes ☐ No. How did you know? `____`
- Did you expect a "Save" button anywhere and not find one (or vice versa)? `____`
- On the Backend page: did the connection / server options make sense, or was it jargon? `____`
- Notes: `________________________________________________`

### Scenario 9: the tools

The wallet ships a set of extra tools. Some live on the **Tools tab**; a few live in the
**⋯ (more) menu** at the top of the wallet. This scenario is about whether people can find
them and understand what they're for; you don't need to be an expert in any of them.

**9a: discover & understand (look, don't use yet).**
- Open the **Tools** tab. For each tool listed, do you understand what it's for from its name + description **without** clicking? Tick the ones that were clear:
  - ☐ Tax report  ☐ Export transactions  ☐ Export descriptor / keys / multisig config  ☐ Address lookup  ☐ PSBT inspector  ☐ Sign & verify message  ☐ Sweep private key  ☐ Test backup
- Which names or descriptions were **jargon / unclear**? (e.g. "PSBT", "descriptor", "WIF", "sweep", "BIP-322") `____`
- Now open the **⋯ menu** at the top of the wallet. Did you expect to find more actions there (Broadcast, Consolidate, Add account)? Was that menu easy to notice? ☐ Yes ☐ No
- Discoverability rating (could you find the tools without being told?): **1 2 3 4 5**

**9b: actually use a couple.** Try at least **two** of these. The organizer will hand you
any inputs you need (a sample to paste, and so on).

- **Sign a message**: sign some text with one of your addresses. Ease: **1 2 3 4 5**. Clear what it's for / how to do it? `____`
- **Address lookup**: check whether an address belongs to this wallet. Ease: **1 2 3 4 5**. `____`
- **Export transactions / Test backup / Tax report**: pick one and run it. Ease: **1 2 3 4 5**. Did you understand the result / file you got? `____`
- **PSBT inspector** (if you're game): paste the sample the organizer gives you and read what it shows. Ease: **1 2 3 4 5**. Could you tell what the transaction would do? `____`

- Did any tool make you nervous it might do something irreversible or risky? Which, and why? `____`
- Overall, did the tools feel like a helpful toolbox or a confusing pile of options? `____`
- Notes: `________________________________________________`

### Scenario 10: when something goes wrong (optional, if it happens)

If you hit an error, a failed sync, a disconnected server, or anything red, pause here.

- What happened, and did the app **tell you clearly** what went wrong and what to do? `____`
- Did you feel stuck, or did you know the next step? `____`
- Ease of recovering: **1 2 3 4 5**

---

## Part 4: rate these overall aspects

Now that you've used it, rate each (1 = poor, 5 = excellent) and add a word on why.

| Aspect | Score (1-5) | Why / what stood out |
|---|---|---|
| **Visual design & polish** (does it look modern & finished?) | | |
| **Language & labels** (plain English vs jargon) | | |
| **Trust & safety** (did it feel safe to hold money in?) | | |
| **Speed & responsiveness** (snappy vs laggy/janky) | | |
| **Consistency** (buttons/screens behave the way you expect) | | |
| **Confidence** (did you generally know what would happen before you clicked?) | | |

- The **single most confusing** moment in the whole session: `____________________`
- The **single best / nicest** moment: `____________________`
- Any word or screen you had to guess the meaning of: `____________________`

---

## Part 5: usability score (SUS)

Ten quick statements. For each, mark how much you agree, **1 = strongly disagree, 5 = strongly agree**. Answer fast, go with your gut.

| # | Statement | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| 1 | I think I would use this wallet regularly | ☐ | ☐ | ☐ | ☐ | ☐ |
| 2 | I found it unnecessarily complex | ☐ | ☐ | ☐ | ☐ | ☐ |
| 3 | I thought it was easy to use | ☐ | ☐ | ☐ | ☐ | ☐ |
| 4 | I'd need help from a techy person to use it | ☐ | ☐ | ☐ | ☐ | ☐ |
| 5 | The different parts of the app fit together well | ☐ | ☐ | ☐ | ☐ | ☐ |
| 6 | There was too much inconsistency | ☐ | ☐ | ☐ | ☐ | ☐ |
| 7 | Most people would learn this very quickly | ☐ | ☐ | ☐ | ☐ | ☐ |
| 8 | It felt awkward / cumbersome to use | ☐ | ☐ | ☐ | ☐ | ☐ |
| 9 | I felt confident using it | ☐ | ☐ | ☐ | ☐ | ☐ |
| 10 | I had to learn a lot before I could get going | ☐ | ☐ | ☐ | ☐ | ☐ |

> **Scoring (organizer):** odd items score `(answer − 1)`; even items score `(5 − answer)`. Sum all ten, multiply by 2.5 for a 0-100 score. Rough read: **>80 excellent, 68 is about average, <50 needs work.**

---

## Part 6: final verdict

- Would you use Corvin to hold your own Bitcoin? ☐ Yes ☐ Maybe ☐ No. Why? `____`
- Would you recommend it to a friend who self-custodies? ☐ Yes ☐ Maybe ☐ No
- If you could change **one thing**, what would it be? `____________________`
- Anything that made you **distrust** the app or hesitate to put money in it? `____`
- Anything that genuinely impressed you? `____`

---

## Part 7: issue log

Jot anything broken, confusing, ugly, or surprising as you go. Tag a severity:

- **Blocker**: couldn't continue / would make me quit / scared me about my money
- **Major**: significant confusion or friction, but I got through it
- **Minor**: small annoyance, didn't really slow me down
- **Cosmetic**: visual nit, typo, alignment, wording

| # | Where (screen) | What happened / what confused me | What I expected | Severity |
|---|---|---|---|---|
| 1 | | | | |
| 2 | | | | |
| 3 | | | | |
| 4 | | | | |
| 5 | | | | |
| 6 | | | | |

---

## Appendix: what the organizer does with the results

- Tally **SUS** across testers for one headline number to track release-over-release.
- Average the **task ease** scores; any task averaging **≤ 3** is a priority fix.
- Group **Issue log** entries by screen; count how many testers hit the same thing (repeat hits are real problems, not one-offs).
- Pull the **"most confusing moment"** answers; those usually point straight at the worst UX debt.
- Watch for **trust** red flags specifically: anything that made a tester hesitate to hold money is the highest-value finding for a self-custody wallet.
