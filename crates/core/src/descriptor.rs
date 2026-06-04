use crate::types::InputKind;
use bitcoin::base58;
use bitcoin::Network;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DescriptorError {
    #[error("empty input")]
    Empty,
    #[error("unrecognised input: {0}")]
    Unrecognised(String),
    #[error("invalid base58: {0}")]
    Base58(#[from] bitcoin::base58::Error),
    #[error("invalid address: {0}")]
    Address(String),
    #[error("descriptor too short for an extended key")]
    TooShort,
    #[error("duplicate xpub: {0}")]
    DuplicateXpub(String),
    #[error("invalid fingerprint '{0}' — expected 8 hex characters")]
    InvalidFingerprint(String),
    #[error("invalid derivation path '{0}'")]
    InvalidPath(String),
    #[error("invalid descriptor: {0}")]
    InvalidDescriptor(String),
}

/// Master-key fingerprint must be exactly 8 hex characters. The BitBox returns
/// it as 4 hex bytes (8 chars); rejecting any other shape stops malformed input
/// from breaking descriptor string parsing further down the stack.
fn validate_fingerprint(fp: &str) -> Result<(), DescriptorError> {
    if fp.len() != 8 || !fp.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DescriptorError::InvalidFingerprint(fp.to_string()));
    }
    Ok(())
}

/// A BIP32 derivation path is a sequence of "/n" or "/n'" segments, optionally
/// prefixed with "m/". Reject anything else before we splice it into the
/// descriptor string — otherwise a malicious caller could break out of the
/// origin brackets.
fn validate_path(path: &str) -> Result<(), DescriptorError> {
    let body = path.trim_start_matches("m/").trim_end_matches('/');
    if body.is_empty() {
        return Err(DescriptorError::InvalidPath(path.to_string()));
    }
    for segment in body.split('/') {
        let num = segment.trim_end_matches('\'').trim_end_matches('h');
        if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
            return Err(DescriptorError::InvalidPath(path.to_string()));
        }
        // BIP32 indices are u31; > 2^31-1 means a hardened-flagged index out of range.
        if num.parse::<u32>().map(|n| n > 0x7FFF_FFFF).unwrap_or(true) {
            return Err(DescriptorError::InvalidPath(path.to_string()));
        }
    }
    Ok(())
}

/// Version bytes for extended public keys (mainnet)
const XPUB_VERSION: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
const YPUB_VERSION: [u8; 4] = [0x04, 0x9D, 0x7C, 0xB2];
const ZPUB_VERSION: [u8; 4] = [0x04, 0xB2, 0x47, 0x46];

/// Testnet extended public key versions
const TPUB_VERSION: [u8; 4] = [0x04, 0x35, 0x87, 0xCF];
const UPUB_VERSION: [u8; 4] = [0x04, 0x4A, 0x52, 0x62];
const VPUB_VERSION: [u8; 4] = [0x04, 0x5F, 0x1C, 0xF6];

pub struct ParsedDescriptor {
    pub kind: InputKind,
    pub external: String,
    pub internal: Option<String>,
}

/// Parse a user-supplied string (address, xpub, ypub, zpub) into BDK descriptor strings.
pub fn parse_input(input: &str, network: Network) -> Result<ParsedDescriptor, DescriptorError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(DescriptorError::Empty);
    }

    // Try to classify by prefix / length heuristics before base58 decode
    if is_address_like(s) {
        // Validate it's a real address
        s.parse::<bitcoin::Address<bitcoin::address::NetworkUnchecked>>()
            .map_err(|e| DescriptorError::Address(e.to_string()))?;
        return Ok(ParsedDescriptor {
            kind: InputKind::Address,
            external: format!("addr({s})"),
            internal: None,
        });
    }

    // Must be an extended key — decode base58check
    let raw = base58::decode_check(s)?;
    if raw.len() < 4 {
        return Err(DescriptorError::TooShort);
    }

    // Safe: the `raw.len() < 4` guard above guarantees at least 4 bytes.
    let version: [u8; 4] = raw[..4].try_into().expect("len checked >= 4");

    match version {
        v if v == XPUB_VERSION || v == TPUB_VERSION => {
            // P2PKH (BIP44)
            Ok(ParsedDescriptor {
                kind: InputKind::Xpub,
                external: format!("pkh({s}/0/*)"),
                internal: Some(format!("pkh({s}/1/*)")),
            })
        }
        v if v == YPUB_VERSION || v == UPUB_VERSION => {
            // P2SH-P2WPKH (BIP49) — convert to xpub first
            let xpub = reversion(&raw, xpub_version(network));
            let xpub_str = base58::encode_check(&xpub);
            Ok(ParsedDescriptor {
                kind: InputKind::Ypub,
                external: format!("sh(wpkh({xpub_str}/0/*))"),
                internal: Some(format!("sh(wpkh({xpub_str}/1/*))")),
            })
        }
        v if v == ZPUB_VERSION || v == VPUB_VERSION => {
            // P2WPKH (BIP84) — convert to xpub first
            let xpub = reversion(&raw, xpub_version(network));
            let xpub_str = base58::encode_check(&xpub);
            Ok(ParsedDescriptor {
                kind: InputKind::Zpub,
                external: format!("wpkh({xpub_str}/0/*)"),
                internal: Some(format!("wpkh({xpub_str}/1/*)")),
            })
        }
        _ => Err(DescriptorError::Unrecognised(s.to_string())),
    }
}

/// Heuristic: does this string look like a Bitcoin address rather than an extended key?
fn is_address_like(s: &str) -> bool {
    // bech32 native segwit / taproot
    let lower = s.to_lowercase();
    if lower.starts_with("bc1") || lower.starts_with("tb1") || lower.starts_with("bcrt1") {
        return true;
    }
    // P2PKH / P2SH: base58, starts with 1, 3, m, n, 2
    // Extended keys start with xpub/ypub/zpub/tpub/upub/vpub
    let ext_prefixes = [
        "xpub", "ypub", "zpub", "xprv", "tpub", "upub", "vpub", "tprv",
    ];
    if ext_prefixes.iter().any(|p| s.starts_with(p)) {
        return false;
    }
    // Short enough to be a legacy address (25–34 chars), not an xpub (111 chars)
    s.len() < 60
}

fn reversion(raw: &[u8], new_version: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    out.extend_from_slice(&new_version);
    out.extend_from_slice(&raw[4..]);
    out
}

fn xpub_version(network: Network) -> [u8; 4] {
    match network {
        Network::Bitcoin => XPUB_VERSION,
        _ => TPUB_VERSION,
    }
}

/// Build descriptors for a hardware-wallet-imported account, embedding the key
/// origin so BDK includes the full derivation path in PSBTs and the HW device
/// can match its own keys when signing.
///
/// `xpub_b58`   — the raw xpub/ypub/zpub returned from the device
/// `fingerprint` — 8 hex chars, the master key fingerprint from the device
/// `path`        — full path as returned by the device, e.g. "m/84'/0'/0'"
/// `account_type` — "native_segwit" | "p2sh_segwit" | "legacy"
pub fn descriptor_from_hw_xpub(
    xpub_b58: &str,
    fingerprint: &str,
    path: &str,
    account_type: &str,
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    validate_fingerprint(fingerprint)?;
    validate_path(path)?;

    let raw = base58::decode_check(xpub_b58.trim())?;
    if raw.len() < 4 {
        return Err(DescriptorError::TooShort);
    }

    // Re-version to plain xpub/tpub so BDK can parse it uniformly.
    let xpub_raw = reversion(&raw, xpub_version(network));
    let xpub_str = base58::encode_check(&xpub_raw);

    // Strip the leading "m/" so the origin looks like [fp/84'/0'/0']
    let path_trimmed = path.trim_start_matches("m/");
    let origin = format!("[{fingerprint}/{path_trimmed}]");

    let (external, internal, kind) = match account_type {
        "p2sh_segwit" => (
            format!("sh(wpkh({origin}{xpub_str}/0/*))"),
            Some(format!("sh(wpkh({origin}{xpub_str}/1/*))")),
            InputKind::Ypub,
        ),
        "legacy" => (
            format!("pkh({origin}{xpub_str}/0/*)"),
            Some(format!("pkh({origin}{xpub_str}/1/*)")),
            InputKind::Xpub,
        ),
        "taproot" => (
            format!("tr({origin}{xpub_str}/0/*)"),
            Some(format!("tr({origin}{xpub_str}/1/*)")),
            InputKind::Taproot,
        ),
        _ => (
            format!("wpkh({origin}{xpub_str}/0/*)"),
            Some(format!("wpkh({origin}{xpub_str}/1/*)")),
            InputKind::Zpub,
        ),
    };

    Ok(ParsedDescriptor {
        kind,
        external,
        internal,
    })
}

// ── Multisig ──────────────────────────────────────────────────────────────────

pub struct MultisigSigner {
    pub fingerprint: String,
    pub path: String,
    pub xpub: String,
}

/// Build a `wsh(sortedmulti(M, [fp/path]xpub/0/*, ...))` descriptor pair for a
/// native-segwit multisig wallet.  BIP67 key sorting is handled by BDK/miniscript.
pub fn descriptor_from_multisig(
    threshold: u32,
    signers: &[MultisigSigner],
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    if signers.is_empty() {
        return Err(DescriptorError::Empty);
    }

    let mut seen_xpubs = std::collections::HashSet::new();
    let mut key_exprs: Vec<String> = Vec::new();
    for signer in signers {
        validate_fingerprint(&signer.fingerprint)?;
        validate_path(&signer.path)?;
        let xpub_trimmed = signer.xpub.trim().to_string();
        if !seen_xpubs.insert(xpub_trimmed.clone()) {
            return Err(DescriptorError::DuplicateXpub(xpub_trimmed));
        }
        let raw = base58::decode_check(&xpub_trimmed)?;
        if raw.len() < 4 {
            return Err(DescriptorError::TooShort);
        }
        let xpub_raw = reversion(&raw, xpub_version(network));
        let xpub_str = base58::encode_check(&xpub_raw);
        let path_trimmed = signer.path.trim_start_matches("m/");
        key_exprs.push(format!(
            "[{}/{}]{}",
            signer.fingerprint, path_trimmed, xpub_str
        ));
    }

    let keys_ext: Vec<String> = key_exprs.iter().map(|k| format!("{k}/0/*")).collect();
    let keys_int: Vec<String> = key_exprs.iter().map(|k| format!("{k}/1/*")).collect();

    let external = format!("wsh(sortedmulti({},{}))", threshold, keys_ext.join(","));
    let internal = format!("wsh(sortedmulti({},{}))", threshold, keys_int.join(","));

    Ok(ParsedDescriptor {
        kind: InputKind::Multisig,
        external,
        internal: Some(internal),
    })
}

// ── Raw descriptor import ───────────────────────────────────────────────────

/// Import a user-pasted output descriptor (e.g. from Sparrow, Core, or a BSMS
/// round-trip). Validates via miniscript, splits BIP-389 multipath descriptors
/// into the receive/change pair BDK wants, and classifies the result onto an
/// existing `InputKind` where it maps cleanly (so an imported multisig lights
/// up the multisig features) — falling back to `InputKind::Descriptor` for
/// miniscript shapes that don't. Returns the descriptor pair plus the multisig
/// threshold when one is present. Network compatibility is enforced downstream
/// when BDK opens the wallet.
pub fn parse_descriptor_import(
    external: &str,
    internal: Option<&str>,
) -> Result<(ParsedDescriptor, Option<u32>), DescriptorError> {
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey};
    use std::str::FromStr;

    let ext_in = external.trim();
    if ext_in.is_empty() {
        return Err(DescriptorError::Empty);
    }

    let parsed = Descriptor::<DescriptorPublicKey>::from_str(ext_in)
        .map_err(|e| DescriptorError::InvalidDescriptor(e.to_string()))?;

    let (ext_desc, int_desc): (
        Descriptor<DescriptorPublicKey>,
        Option<Descriptor<DescriptorPublicKey>>,
    ) = if parsed.is_multipath() {
        // BIP-389 multipath `<0;1>`: split into one descriptor per path; first
        // is the receive keychain, second the change.
        let singles = parsed
            .into_single_descriptors()
            .map_err(|e| DescriptorError::InvalidDescriptor(e.to_string()))?;
        let mut it = singles.into_iter();
        let ext = it.next().ok_or_else(|| {
            DescriptorError::InvalidDescriptor(
                "multipath descriptor produced no sub-descriptors".into(),
            )
        })?;
        (ext, it.next())
    } else if let Some(int_str) = internal.map(str::trim).filter(|s| !s.is_empty()) {
        let int = Descriptor::<DescriptorPublicKey>::from_str(int_str)
            .map_err(|e| DescriptorError::InvalidDescriptor(e.to_string()))?;
        (parsed, Some(int))
    } else {
        // No change descriptor: derive one by swapping the last `/0/*` for
        // `/1/*`. If that doesn't apply, fall back to a single-keychain wallet
        // (change reuses the receive descriptor) rather than guess.
        let derived = derive_change_descriptor(&parsed);
        (parsed, derived)
    };

    let (kind, threshold) = classify_descriptor(&ext_desc);

    Ok((
        ParsedDescriptor {
            kind,
            external: ext_desc.to_string(),
            internal: int_desc.map(|d| d.to_string()),
        },
        threshold,
    ))
}

fn derive_change_descriptor(
    ext: &bdk_wallet::miniscript::Descriptor<bdk_wallet::miniscript::DescriptorPublicKey>,
) -> Option<bdk_wallet::miniscript::Descriptor<bdk_wallet::miniscript::DescriptorPublicKey>> {
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey};
    use std::str::FromStr;

    let s = ext.to_string();
    let body = s.split('#').next().unwrap_or(&s);
    // Advance every key's receive chain (`/0/*`) to the change chain (`/1/*`) —
    // all of them, not just the last, or a multisig descriptor ends up with
    // mismatched per-key chains. No swap possible → single-keychain wallet.
    let swapped = body.replace("/0/*", "/1/*");
    if swapped == body {
        return None;
    }
    Descriptor::<DescriptorPublicKey>::from_str(&swapped).ok()
}

/// Map a validated descriptor onto an `InputKind`. Non-taproot multisig (sorted
/// or not) is detected so the multisig features light up; bare single-sig maps
/// to its script type; everything else is a raw `Descriptor`.
fn classify_descriptor(
    desc: &bdk_wallet::miniscript::Descriptor<bdk_wallet::miniscript::DescriptorPublicKey>,
) -> (InputKind, Option<u32>) {
    use bdk_wallet::miniscript::descriptor::DescriptorType;

    // Taproot first — even `tr(…,multi_a(…))`. The multisig combine/info paths
    // assume `wsh` and ECDSA `partial_sigs`; taproot sigs live in different PSBT
    // fields, so routing taproot into Multisig would surface broken actions.
    if desc.desc_type() == DescriptorType::Tr {
        return (InputKind::Taproot, None);
    }
    if let Some(k) = extract_multi_threshold(&desc.to_string()) {
        return (InputKind::Multisig, Some(k));
    }
    match desc.desc_type() {
        DescriptorType::Wpkh => (InputKind::Zpub, None),
        DescriptorType::ShWpkh => (InputKind::Ypub, None),
        DescriptorType::Pkh => (InputKind::Xpub, None),
        _ => (InputKind::Descriptor, None),
    }
}

/// Pull the threshold `M` out of a `multi`/`sortedmulti` fragment in a (already
/// validated, non-taproot) descriptor string. `sortedmulti(M,…)` contains the
/// `multi(` substring, so the one pattern covers both forms.
fn extract_multi_threshold(s: &str) -> Option<u32> {
    let idx = s.find("multi(")?;
    let rest = &s[idx + "multi(".len()..];
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse::<u32>().ok()
}

// ── Inheritance / recovery vault builder ────────────────────────────────────

/// The `[fingerprint/path]xpub` origin expression (no chain/index suffix),
/// shared by the multisig + vault builders.
fn key_origin_expr(signer: &MultisigSigner, network: Network) -> Result<String, DescriptorError> {
    validate_fingerprint(&signer.fingerprint)?;
    validate_path(&signer.path)?;
    let raw = base58::decode_check(signer.xpub.trim())?;
    if raw.len() < 4 {
        return Err(DescriptorError::TooShort);
    }
    let xpub_str = base58::encode_check(&reversion(&raw, xpub_version(network)));
    let path_trimmed = signer.path.trim_start_matches("m/");
    Ok(format!(
        "[{}/{}]{}",
        signer.fingerprint, path_trimmed, xpub_str
    ))
}

/// Timelock guarding the recovery branch of a vault.
pub enum VaultTimelock {
    /// Relative (CSV `older`) in blocks. Max 65535 (~1.25 years).
    RelativeBlocks(u32),
    /// Absolute (CLTV `after`) block height — use for horizons beyond ~1.25y.
    AbsoluteHeight(u32),
}

/// Build a `wsh(or_d(<primary>, and_v(v:<recovery>, <timelock>)))` descriptor:
/// the primary keys (N-of-M, or a single key) can spend anytime; the recovery
/// keys (k-of-j, or a single key) can spend only after the timelock. The
/// recovery set must be disjoint from the primary set (reused keys make the
/// descriptor non-standard). Segwit-v0 (wsh) for broad signer support;
/// validated by parsing it back through miniscript.
pub fn descriptor_from_inheritance_vault(
    threshold: u32,
    primary: &[MultisigSigner],
    recovery_threshold: u32,
    recovery: &[MultisigSigner],
    timelock: VaultTimelock,
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    if primary.is_empty() {
        return Err(DescriptorError::Empty);
    }
    if threshold < 1 || threshold as usize > primary.len() {
        return Err(DescriptorError::InvalidDescriptor(format!(
            "primary threshold {threshold} is out of range for {} primary signer(s)",
            primary.len()
        )));
    }
    if recovery.is_empty() {
        return Err(DescriptorError::InvalidDescriptor(
            "a vault needs at least one recovery key".into(),
        ));
    }
    if recovery_threshold < 1 || recovery_threshold as usize > recovery.len() {
        return Err(DescriptorError::InvalidDescriptor(format!(
            "recovery threshold {recovery_threshold} is out of range for {} recovery key(s)",
            recovery.len()
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let mut primary_exprs = Vec::new();
    for s in primary {
        if !seen.insert(s.xpub.trim().to_string()) {
            return Err(DescriptorError::DuplicateXpub(s.xpub.trim().to_string()));
        }
        primary_exprs.push(key_origin_expr(s, network)?);
    }
    let mut recovery_exprs = Vec::new();
    for s in recovery {
        if !seen.insert(s.xpub.trim().to_string()) {
            return Err(DescriptorError::DuplicateXpub(s.xpub.trim().to_string()));
        }
        recovery_exprs.push(key_origin_expr(s, network)?);
    }

    let timelock_frag = timelock_fragment(&timelock)?;

    let multi_or_pk = |exprs: &[String], k: u32, chain: u32| -> String {
        if exprs.len() == 1 {
            format!("pk({}/{chain}/*)", exprs[0])
        } else {
            let keys: Vec<String> = exprs.iter().map(|e| format!("{e}/{chain}/*")).collect();
            format!("multi({k},{})", keys.join(","))
        }
    };

    let build = |chain: u32| -> String {
        let primary_frag = multi_or_pk(&primary_exprs, threshold, chain);
        let recovery_frag = multi_or_pk(&recovery_exprs, recovery_threshold, chain);
        format!("wsh(or_d({primary_frag},and_v(v:{recovery_frag},{timelock_frag})))")
    };

    let external = build(0);
    let internal = build(1);
    validate_descriptor(&external)?;
    Ok(ParsedDescriptor {
        kind: InputKind::Descriptor,
        external,
        internal: Some(internal),
    })
}

/// Shared CSV/CLTV fragment for the policy templates.
fn timelock_fragment(timelock: &VaultTimelock) -> Result<String, DescriptorError> {
    match timelock {
        VaultTimelock::RelativeBlocks(n) => {
            if *n == 0 || *n > 0xffff {
                return Err(DescriptorError::InvalidDescriptor(
                    "relative timelock must be 1..=65535 blocks (~1.25 years max)".into(),
                ));
            }
            Ok(format!("older({n})"))
        }
        VaultTimelock::AbsoluteHeight(h) => {
            if *h == 0 || *h >= 500_000_000 {
                return Err(DescriptorError::InvalidDescriptor(
                    "absolute block-height timelock must be 1..500000000".into(),
                ));
            }
            Ok(format!("after({h})"))
        }
    }
}

/// Round-trip a freshly-built receive descriptor through the miniscript parser
/// so malformed fragments fail here rather than at BDK wallet creation.
fn validate_descriptor(external: &str) -> Result<(), DescriptorError> {
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey};
    use std::str::FromStr;
    Descriptor::<DescriptorPublicKey>::from_str(external).map_err(|e| {
        DescriptorError::InvalidDescriptor(format!("policy descriptor failed to validate: {e}"))
    })?;
    Ok(())
}

/// Timelocked savings: `wsh(and_v(v:pk(K), <timelock>))`. A single key whose
/// funds are unspendable until a relative delay elapses or an absolute block
/// height is reached.
pub fn descriptor_from_timelocked_savings(
    signer: &MultisigSigner,
    timelock: VaultTimelock,
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    let key = key_origin_expr(signer, network)?;
    let timelock_frag = timelock_fragment(&timelock)?;
    let build =
        |chain: u32| -> String { format!("wsh(and_v(v:pk({key}/{chain}/*),{timelock_frag}))") };
    let external = build(0);
    let internal = build(1);
    validate_descriptor(&external)?;
    Ok(ParsedDescriptor {
        kind: InputKind::Descriptor,
        external,
        internal: Some(internal),
    })
}

/// BIP-341 suggested NUMS point (x-only). No known discrete log, so it's a
/// provably-unspendable internal key — used when a taproot descriptor has no
/// sensible key-path and every spend must go through a tapleaf.
const NUMS_XONLY: &str = "50929b74c1a04954b78b4b6035e97a5e078a5a0f28ec96d547bfee9ace803ac0";

/// `pk(K/chain/*)` for one key, else `multi_a(k, …)` — the taproot (tapscript)
/// flavor of a threshold, using CHECKSIGADD rather than wsh `multi`.
fn tap_multi_or_pk(exprs: &[String], k: u32, chain: u32) -> String {
    if exprs.len() == 1 {
        format!("pk({}/{chain}/*)", exprs[0])
    } else {
        let keys: Vec<String> = exprs.iter().map(|e| format!("{e}/{chain}/*")).collect();
        format!("multi_a({k},{})", keys.join(","))
    }
}

/// Taproot inheritance vault: `tr(PRIMARY, and_v(v:<recovery>, <timelock>))`.
/// The single primary key spends anytime via the **key-path** (cheap, private,
/// indistinguishable from a normal taproot payment); a disjoint recovery group
/// (k-of-j, distinct keys) spends only after the timelock via a hidden tapleaf.
/// Primary is a single key because the key-path can't be multi-key without
/// MuSig2 — callers must enforce that.
pub fn descriptor_from_taproot_vault(
    primary: &MultisigSigner,
    recovery_threshold: u32,
    recovery: &[MultisigSigner],
    timelock: VaultTimelock,
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    if recovery.is_empty() {
        return Err(DescriptorError::InvalidDescriptor(
            "a vault needs at least one recovery key".into(),
        ));
    }
    if recovery_threshold < 1 || recovery_threshold as usize > recovery.len() {
        return Err(DescriptorError::InvalidDescriptor(format!(
            "recovery threshold {recovery_threshold} is out of range for {} recovery key(s)",
            recovery.len()
        )));
    }

    let mut seen = std::collections::HashSet::new();
    seen.insert(primary.xpub.trim().to_string());
    let primary_expr = key_origin_expr(primary, network)?;
    let mut recovery_exprs = Vec::new();
    for s in recovery {
        if !seen.insert(s.xpub.trim().to_string()) {
            return Err(DescriptorError::DuplicateXpub(s.xpub.trim().to_string()));
        }
        recovery_exprs.push(key_origin_expr(s, network)?);
    }
    let timelock_frag = timelock_fragment(&timelock)?;

    let build = |chain: u32| -> String {
        let recovery_frag = tap_multi_or_pk(&recovery_exprs, recovery_threshold, chain);
        // Single tapleaf → no braces (braces denote a 2-child branch).
        format!("tr({primary_expr}/{chain}/*,and_v(v:{recovery_frag},{timelock_frag}))")
    };

    let external = build(0);
    let internal = build(1);
    validate_descriptor(&external)?;
    Ok(ParsedDescriptor {
        kind: InputKind::Descriptor,
        external,
        internal: Some(internal),
    })
}

/// Taproot timelocked savings: `tr(NUMS, and_v(v:pk(K), <timelock>))`. The
/// key-path is the unspendable NUMS point (a key-path can't enforce a
/// timelock), so the only spend is the timelocked tapleaf — hidden until used.
pub fn descriptor_from_taproot_savings(
    signer: &MultisigSigner,
    timelock: VaultTimelock,
    network: Network,
) -> Result<ParsedDescriptor, DescriptorError> {
    let key = key_origin_expr(signer, network)?;
    let timelock_frag = timelock_fragment(&timelock)?;
    let build = |chain: u32| -> String {
        format!("tr({NUMS_XONLY},and_v(v:pk({key}/{chain}/*),{timelock_frag}))")
    };
    let external = build(0);
    let internal = build(1);
    validate_descriptor(&external)?;
    Ok(ParsedDescriptor {
        kind: InputKind::Descriptor,
        external,
        internal: Some(internal),
    })
}

/// Combine a wallet's external (`/0/*`) descriptor into the portable
/// **multipath** form (`/<0;1>/*`) with a checksum — the standard import
/// format for Coldcard, Sparrow, Bitcoin Core, etc. (one descriptor that
/// carries both the receive and change keychains).
pub fn descriptor_to_multipath(external: &str) -> Result<String, DescriptorError> {
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey};
    use std::str::FromStr;

    // Drop any existing #checksum before rewriting.
    let body = match external.trim().rsplit_once('#') {
        Some((b, sum)) if sum.len() == 8 && sum.chars().all(|c| c.is_ascii_alphanumeric()) => b,
        _ => external.trim(),
    };
    let multi = body.replace("/0/*", "/<0;1>/*");
    // Re-parse to validate and emit a canonical string with a fresh checksum.
    let desc = Descriptor::<DescriptorPublicKey>::from_str(&multi)
        .map_err(|e| DescriptorError::InvalidDescriptor(format!("multipath export failed: {e}")))?;
    Ok(desc.to_string())
}

// ── Policy description (miniscript → human-readable summary) ─────────────────

/// A spending-policy summary lifted from a (miniscript) descriptor — used to
/// make imported policy wallets and created vaults legible.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicySummary {
    /// The canonical miniscript *policy* string (e.g.
    /// `or(and(older(26280),pk(<recovery>)),thresh(2,pk(..),pk(..),pk(..)))`).
    pub policy: String,
    /// Every timelock in the policy — the part users care about for vaults.
    pub timelocks: Vec<PolicyTimelock>,
    /// Master fingerprints of the keys in the policy (deduped, sorted).
    pub key_fingerprints: Vec<String>,
    /// True when the policy offers a spending *choice* — i.e. an OR/threshold
    /// with a non-trivial branch (a vault: primary OR timelocked recovery).
    /// False for a single-path policy (timelocked savings = one AND branch).
    /// Lets the UI show a primary/recovery selector for vaults but not savings.
    pub requires_path: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyTimelock {
    /// "relative" (`older`, CSV) or "absolute" (`after`, CLTV).
    pub kind: String,
    /// Raw consensus value.
    pub value: u32,
    /// true = block-height/block-count based; false = time based.
    pub blocks: bool,
    /// Humanized label, e.g. "26280 blocks (≈6 months)" or "block 850000".
    pub label: String,
}

fn humanize_blocks(blocks: u32) -> String {
    let days = (blocks as f64 * 10.0) / 1440.0; // ~10 min/block
    let approx = if days >= 365.0 {
        format!("≈{:.1} years", days / 365.0)
    } else if days >= 30.0 {
        format!("≈{:.0} months", days / 30.0)
    } else if days >= 1.0 {
        format!("≈{:.0} days", days)
    } else {
        format!("≈{:.0} hours", days * 24.0)
    };
    format!("{blocks} blocks ({approx})")
}

/// Lift a descriptor to its semantic policy and summarize it. Multipath
/// descriptors are summarized on their receive branch.
pub fn describe_policy(descriptor: &str) -> Result<PolicySummary, DescriptorError> {
    use bdk_wallet::miniscript::policy::semantic::Policy as SemPolicy;
    use bdk_wallet::miniscript::policy::Liftable;
    use bdk_wallet::miniscript::{Descriptor, DescriptorPublicKey};
    use std::str::FromStr;

    let parsed = Descriptor::<DescriptorPublicKey>::from_str(descriptor.trim())
        .map_err(|e| DescriptorError::InvalidDescriptor(e.to_string()))?;
    let desc = if parsed.is_multipath() {
        parsed
            .into_single_descriptors()
            .map_err(|e| DescriptorError::InvalidDescriptor(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                DescriptorError::InvalidDescriptor("empty multipath descriptor".into())
            })?
    } else {
        parsed
    };

    let policy: SemPolicy<DescriptorPublicKey> = desc
        .lift()
        .map_err(|e| DescriptorError::InvalidDescriptor(format!("can't lift to policy: {e}")))?;

    let mut timelocks = Vec::new();
    let mut fps = Vec::new();
    let mut requires_path = false;
    walk_policy(&policy, &mut timelocks, &mut fps, &mut requires_path);
    fps.sort();
    fps.dedup();

    Ok(PolicySummary {
        policy: policy.to_string(),
        timelocks,
        key_fingerprints: fps,
        requires_path,
    })
}

/// Can a UTXO with `confirmations` confs, on a chain at `tip`, satisfy this
/// timelock? `Some(false)` = locked, `Some(true)` = unlocked, `None` = we can't
/// pre-judge (time-based locks) so leave it to BDK/consensus. Single source of
/// the maturity rule for both the spend gate and the UI.
pub fn timelock_spendable(t: &PolicyTimelock, confirmations: u32, tip: u32) -> Option<bool> {
    match (t.kind.as_str(), t.blocks) {
        // Relative block count (CSV): needs `value` confirmations.
        ("relative", true) => Some(confirmations >= t.value),
        // Absolute block height (CLTV): spendable once the tip reaches it.
        ("absolute", true) => Some(tip >= t.value),
        // Time-based locks — don't pre-gate; let the build/network decide.
        _ => None,
    }
}

fn walk_policy(
    p: &bdk_wallet::miniscript::policy::semantic::Policy<
        bdk_wallet::miniscript::DescriptorPublicKey,
    >,
    timelocks: &mut Vec<PolicyTimelock>,
    fps: &mut Vec<String>,
    requires_path: &mut bool,
) {
    use bdk_wallet::miniscript::policy::semantic::Policy as P;
    match p {
        P::Key(pk) => fps.push(pk.master_fingerprint().to_string()),
        P::Older(rel) => {
            let value = rel.to_consensus_u32();
            let blocks = rel.is_height_locked();
            let label = if blocks {
                humanize_blocks(value & 0xffff)
            } else {
                let secs = (value & 0xffff) as u64 * 512;
                format!("{} (~{} days)", secs, secs / 86400)
            };
            timelocks.push(PolicyTimelock {
                kind: "relative".into(),
                value,
                blocks,
                label,
            });
        }
        P::After(abs) => {
            let value = abs.to_consensus_u32();
            let blocks = abs.is_block_height();
            let label = if blocks {
                format!("block {value}")
            } else {
                format!("unix time {value}")
            };
            timelocks.push(PolicyTimelock {
                kind: "absolute".into(),
                value,
                blocks,
                label,
            });
        }
        P::Thresh(t) => {
            // A threshold with a real choice (not a pure AND, k==n) means the
            // policy has multiple spending paths — a vault, not savings.
            if !t.is_and() {
                *requires_path = true;
            }
            for sub in t.iter() {
                walk_policy(sub, timelocks, fps, requires_path);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bech32_address() {
        let result = parse_input(
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
            Network::Bitcoin,
        );
        assert!(result.is_ok());
        let p = result.unwrap();
        assert_eq!(p.kind, InputKind::Address);
        assert!(p.external.starts_with("addr("));
        assert!(p.internal.is_none());
    }

    #[test]
    fn test_empty_input() {
        assert!(matches!(
            parse_input("", Network::Bitcoin),
            Err(DescriptorError::Empty)
        ));
    }

    fn test_xpub(seed_byte: u8) -> String {
        use bitcoin::bip32::{Xpriv, Xpub};
        let secp = bitcoin::secp256k1::Secp256k1::new();
        let xpriv = Xpriv::new_master(Network::Bitcoin, &[seed_byte; 32]).unwrap();
        Xpub::from_priv(&secp, &xpriv).to_string()
    }

    #[test]
    fn import_single_sig_wpkh() {
        let (p, thr) =
            parse_descriptor_import(&format!("wpkh({}/0/*)", test_xpub(1)), None).unwrap();
        assert_eq!(p.kind, InputKind::Zpub);
        assert_eq!(thr, None);
        // Change derived by swapping /0/* -> /1/*.
        assert!(p.internal.as_ref().unwrap().contains("/1/*"));
    }

    #[test]
    fn import_taproot() {
        let (p, _) = parse_descriptor_import(&format!("tr({}/0/*)", test_xpub(1)), None).unwrap();
        assert_eq!(p.kind, InputKind::Taproot);
    }

    #[test]
    fn import_sortedmulti_sets_threshold() {
        let desc = format!(
            "wsh(sortedmulti(2,{}/0/*,{}/0/*))",
            test_xpub(1),
            test_xpub(2)
        );
        let (p, thr) = parse_descriptor_import(&desc, None).unwrap();
        assert_eq!(p.kind, InputKind::Multisig);
        assert_eq!(thr, Some(2));
    }

    #[test]
    fn import_taproot_multisig_is_not_wsh_multisig() {
        // tr(internal, {multi_a(...)}): combine/info can't handle taproot sigs,
        // so it must NOT be classified Multisig (would surface broken actions).
        let desc = format!(
            "tr({}/0/*,multi_a(2,{}/0/*,{}/0/*))",
            test_xpub(1),
            test_xpub(2),
            test_xpub(3)
        );
        let (p, thr) = parse_descriptor_import(&desc, None).unwrap();
        assert_eq!(p.kind, InputKind::Taproot);
        assert_eq!(thr, None);
    }

    #[test]
    fn import_multisig_external_only_derives_all_change_chains() {
        // External-only multisig: BOTH keys must advance /0/* -> /1/* in the
        // derived change descriptor, not just the last one.
        let desc = format!(
            "wsh(sortedmulti(2,{}/0/*,{}/0/*))",
            test_xpub(1),
            test_xpub(2)
        );
        let (p, _) = parse_descriptor_import(&desc, None).unwrap();
        let internal = p.internal.expect("change descriptor derived");
        assert!(
            !internal.contains("/0/*"),
            "no key should be left on the receive chain: {internal}"
        );
        assert_eq!(
            internal.matches("/1/*").count(),
            2,
            "both keys advance to change chain"
        );
    }

    #[test]
    fn import_multipath_splits() {
        let (p, _) =
            parse_descriptor_import(&format!("wpkh({}/<0;1>/*)", test_xpub(1)), None).unwrap();
        assert!(p.internal.is_some());
        assert!(p.external.contains("/0/*"));
        assert!(p.internal.as_ref().unwrap().contains("/1/*"));
    }

    #[test]
    fn import_rejects_garbage() {
        assert!(parse_descriptor_import("not a descriptor", None).is_err());
    }

    #[test]
    fn describe_policy_extracts_timelock_and_keys() {
        // A vault-ish policy: key A anytime, OR key B after 26280 blocks.
        let desc = format!(
            "wsh(or_d(pk({}/0/*),and_v(v:pk({}/0/*),older(26280))))",
            test_xpub(1),
            test_xpub(2)
        );
        let s = describe_policy(&desc).unwrap();
        assert_eq!(s.timelocks.len(), 1);
        assert_eq!(s.timelocks[0].kind, "relative");
        assert_eq!(s.timelocks[0].value, 26280);
        assert!(s.timelocks[0].blocks);
        assert!(
            s.timelocks[0].label.contains("months"),
            "humanized: {}",
            s.timelocks[0].label
        );
        assert_eq!(s.key_fingerprints.len(), 2);
        assert!(s.policy.contains("older(26280)"), "policy: {}", s.policy);
    }

    fn signer(seed: u8, path: &str) -> MultisigSigner {
        MultisigSigner {
            fingerprint: format!("{:08x}", 0x1000_0000u32 + seed as u32),
            path: path.to_string(),
            xpub: test_xpub(seed),
        }
    }

    #[test]
    fn vault_single_primary_relative_timelock() {
        let p = [signer(1, "m/84'/0'/0'")];
        let r = [signer(2, "m/84'/0'/0'")];
        let v = descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(26280),
            Network::Bitcoin,
        )
        .unwrap();
        assert_eq!(v.kind, InputKind::Descriptor);
        assert!(v.external.starts_with("wsh(or_d(pk("));
        assert!(v.external.contains("older(26280)"));
        assert!(v.internal.as_ref().unwrap().contains("/1/*"));
        // The generated descriptor describes as a policy with that timelock.
        let s = describe_policy(&v.external).unwrap();
        assert_eq!(s.timelocks.len(), 1);
        assert_eq!(s.timelocks[0].value, 26280);
    }

    #[test]
    fn vault_multisig_primary_absolute_timelock() {
        let p = [signer(1, "m/48'/0'/0'/2'"), signer(2, "m/48'/0'/0'/2'")];
        let r = [signer(3, "m/84'/0'/0'")];
        let v = descriptor_from_inheritance_vault(
            2,
            &p,
            1,
            &r,
            VaultTimelock::AbsoluteHeight(900_000),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(v.external.contains("multi(2,"));
        assert!(v.external.contains("after(900000)"));
        let s = describe_policy(&v.external).unwrap();
        assert_eq!(s.timelocks[0].kind, "absolute");
        assert_eq!(s.key_fingerprints.len(), 3);
    }

    #[test]
    fn vault_multi_key_recovery_group_bdk_accepts() {
        // 2-of-3 primary now, OR 2-of-3 *distinct* recovery keys after timelock.
        let p = [
            signer(1, "m/48'/0'/0'/2'"),
            signer(2, "m/48'/0'/0'/2'"),
            signer(3, "m/48'/0'/0'/2'"),
        ];
        let r = [
            signer(4, "m/48'/0'/0'/2'"),
            signer(5, "m/48'/0'/0'/2'"),
            signer(6, "m/48'/0'/0'/2'"),
        ];
        let v = descriptor_from_inheritance_vault(
            2,
            &p,
            2,
            &r,
            VaultTimelock::RelativeBlocks(52560),
            Network::Bitcoin,
        )
        .unwrap();
        assert_eq!(
            v.external.matches("multi(2,").count(),
            2,
            "both primary and recovery are 2-of-3: {}",
            v.external
        );
        let s = describe_policy(&v.external).unwrap();
        assert_eq!(s.timelocks[0].value, 52560);
        assert_eq!(s.key_fingerprints.len(), 6);
        use bdk_wallet::Wallet;
        let res = Wallet::create(v.external.clone(), v.internal.clone().unwrap())
            .network(Network::Bitcoin)
            .create_wallet_no_persist();
        assert!(
            res.is_ok(),
            "BDK must accept a multi-key recovery vault: {res:?}"
        );
    }

    #[test]
    fn vault_rejects_bad_threshold_and_dupes() {
        let p = [signer(1, "m/84'/0'/0'")];
        let r = [signer(2, "m/84'/0'/0'")];
        // primary threshold out of range
        assert!(descriptor_from_inheritance_vault(
            2,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin
        )
        .is_err());
        // recovery key reuses a primary key
        let dupe = [signer(1, "m/84'/0'/0'")];
        assert!(descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &dupe,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin
        )
        .is_err());
        // recovery threshold out of range for the recovery set
        assert!(descriptor_from_inheritance_vault(
            1,
            &p,
            2,
            &r,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin
        )
        .is_err());
        // timelock too large
        assert!(descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(70000),
            Network::Bitcoin
        )
        .is_err());
    }

    #[test]
    fn describe_policy_plain_multisig_has_no_timelocks() {
        let desc = format!("wsh(multi(2,{}/0/*,{}/0/*))", test_xpub(1), test_xpub(2));
        let s = describe_policy(&desc).unwrap();
        assert!(s.timelocks.is_empty());
        assert_eq!(s.key_fingerprints.len(), 2);
    }

    #[test]
    fn timelocked_savings_builds_and_describes() {
        let s = signer(1, "m/84'/0'/0'");
        let v = descriptor_from_timelocked_savings(
            &s,
            VaultTimelock::AbsoluteHeight(1_000_000),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(v.external.starts_with("wsh(and_v(v:pk("));
        assert!(v.external.contains("after(1000000)"));
        assert!(v.internal.as_ref().unwrap().contains("/1/*"));
        let sum = describe_policy(&v.external).unwrap();
        assert_eq!(sum.timelocks.len(), 1);
        assert_eq!(sum.timelocks[0].kind, "absolute");
        use bdk_wallet::Wallet;
        let res = Wallet::create(v.external.clone(), v.internal.clone().unwrap())
            .network(Network::Bitcoin)
            .create_wallet_no_persist();
        assert!(
            res.is_ok(),
            "BDK must accept a timelocked-savings descriptor: {res:?}"
        );
    }

    fn bdk_opens(p: &ParsedDescriptor) -> bool {
        use bdk_wallet::Wallet;
        Wallet::create(p.external.clone(), p.internal.clone().unwrap())
            .network(Network::Bitcoin)
            .create_wallet_no_persist()
            .is_ok()
    }

    #[test]
    fn taproot_vault_single_recovery_keypath_and_leaf() {
        let primary = signer(1, "m/86'/0'/0'");
        let recovery = [signer(2, "m/86'/0'/0'")];
        let v = descriptor_from_taproot_vault(
            &primary,
            1,
            &recovery,
            VaultTimelock::RelativeBlocks(26280),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(v.external.starts_with("tr("));
        assert!(v.external.contains("older(26280)"));
        assert!(v.internal.as_ref().unwrap().contains("/1/*"));
        assert!(bdk_opens(&v), "BDK must open the taproot vault");
        let s = describe_policy(&v.external).unwrap();
        assert_eq!(s.timelocks[0].value, 26280);
    }

    #[test]
    fn taproot_vault_multi_a_recovery_group() {
        let primary = signer(1, "m/86'/0'/0'");
        let recovery = [signer(2, "m/86'/0'/0'"), signer(3, "m/86'/0'/0'")];
        let v = descriptor_from_taproot_vault(
            &primary,
            2,
            &recovery,
            VaultTimelock::AbsoluteHeight(1_000_000),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(v.external.contains("multi_a(2,"));
        assert!(v.external.contains("after(1000000)"));
        assert!(bdk_opens(&v), "BDK must open the multi_a taproot vault");
    }

    #[test]
    fn taproot_vault_rejects_dupe_and_bad_recovery_threshold() {
        let primary = signer(1, "m/86'/0'/0'");
        // recovery reuses the primary key
        let dupe = [signer(1, "m/86'/0'/0'")];
        assert!(descriptor_from_taproot_vault(
            &primary,
            1,
            &dupe,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin
        )
        .is_err());
        // recovery threshold out of range
        let r = [signer(2, "m/86'/0'/0'")];
        assert!(descriptor_from_taproot_vault(
            &primary,
            2,
            &r,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin
        )
        .is_err());
    }

    #[test]
    fn taproot_savings_uses_nums_internal_and_opens() {
        let s = signer(4, "m/86'/0'/0'");
        let v = descriptor_from_taproot_savings(
            &s,
            VaultTimelock::RelativeBlocks(1000),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(
            v.external.starts_with(&format!("tr({NUMS_XONLY}")),
            "NUMS internal key: {}",
            v.external
        );
        assert!(v.external.contains("older(1000)"));
        assert!(
            bdk_opens(&v),
            "BDK must open the taproot savings descriptor"
        );
        let sum = describe_policy(&v.external).unwrap();
        assert_eq!(sum.timelocks[0].value, 1000);
    }

    fn test_tprv(seed_byte: u8) -> String {
        use bitcoin::bip32::Xpriv;
        Xpriv::new_master(Network::Regtest, &[seed_byte; 32])
            .unwrap()
            .to_string()
    }

    #[test]
    fn vault_spends_via_primary_and_recovery_paths() {
        // The network-free proof that a vault is actually spendable: fund an
        // xpriv-backed vault in-memory, then build→sign→finalize through BOTH
        // the primary and the recovery branch using `policy_path` exactly as
        // send.rs does. Catches the SpendingPolicyRequired gap.
        use bdk_wallet::bitcoin::{hashes::Hash, Amount, BlockHash, FeeRate, Network as N};
        use bdk_wallet::chain::BlockId;
        use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
        use bdk_wallet::{KeychainKind, SignOptions, Wallet};
        use std::collections::BTreeMap;

        let a = test_tprv(1);
        let b = test_tprv(2);
        // Primary key A spends now; recovery key B spends after older(10).
        let ext = format!("wsh(or_d(pk({a}/0/*),and_v(v:pk({b}/0/*),older(10))))");
        let int = format!("wsh(or_d(pk({a}/1/*),and_v(v:pk({b}/1/*),older(10))))");

        let fund = |branch: usize, current_height: u32| -> bool {
            let mut w = Wallet::create(ext.clone(), int.clone())
                .network(N::Regtest)
                .create_wallet_no_persist()
                .unwrap();
            insert_checkpoint(
                &mut w,
                BlockId {
                    height: 1_000,
                    hash: BlockHash::all_zeros(),
                },
            );
            let _ = receive_output_in_latest_block(&mut w, Amount::from_sat(100_000));
            let dest = w.reveal_next_address(KeychainKind::Internal).address;
            // Both keychains carry the OR policy — change (internal) needs the
            // path too, or finish() errors SpendingPolicyRequired(Internal).
            let ext_id = w.policies(KeychainKind::External).unwrap().unwrap().id;
            let int_id = w.policies(KeychainKind::Internal).unwrap().unwrap().id;

            let mut builder = w.build_tx();
            builder.fee_rate(FeeRate::from_sat_per_vb(2).unwrap());
            builder.add_recipient(dest.script_pubkey(), Amount::from_sat(40_000));
            builder.policy_path(
                BTreeMap::from([(ext_id, vec![branch])]),
                KeychainKind::External,
            );
            builder.policy_path(
                BTreeMap::from([(int_id, vec![branch])]),
                KeychainKind::Internal,
            );
            builder.current_height(current_height);
            let mut psbt = builder
                .finish()
                .expect("vault PSBT builds with policy_path");
            w.sign(&mut psbt, SignOptions::default()).expect("sign")
        };

        // Primary: spendable immediately.
        assert!(fund(0, 1_000), "primary path must finalize");
        // Recovery: only after older(10) — at height 1000 (funded) + 10 = 1010.
        assert!(
            fund(1, 1_010),
            "recovery path must finalize once the timelock elapses"
        );
    }

    #[test]
    fn requires_path_distinguishes_vault_from_savings() {
        // Vault (or_d) → has a choice → requires_path true.
        let p = [signer(1, "m/84'/0'/0'")];
        let r = [signer(2, "m/84'/0'/0'")];
        let vault = descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(
            describe_policy(&vault.external).unwrap().requires_path,
            "vault offers a choice"
        );
        // Savings (and_v, single path) → no choice → requires_path false.
        let s = signer(3, "m/84'/0'/0'");
        let sav = descriptor_from_timelocked_savings(
            &s,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin,
        )
        .unwrap();
        assert!(
            !describe_policy(&sav.external).unwrap().requires_path,
            "savings is a single path"
        );
    }

    #[test]
    fn timelock_spendable_rules() {
        let rel = PolicyTimelock {
            kind: "relative".into(),
            value: 10,
            blocks: true,
            label: String::new(),
        };
        assert_eq!(
            timelock_spendable(&rel, 9, 1_000),
            Some(false),
            "9 confs < 10"
        );
        assert_eq!(
            timelock_spendable(&rel, 10, 1_000),
            Some(true),
            "10 confs == 10 → spendable"
        );
        let abs = PolicyTimelock {
            kind: "absolute".into(),
            value: 900_000,
            blocks: true,
            label: String::new(),
        };
        assert_eq!(
            timelock_spendable(&abs, 5, 899_999),
            Some(false),
            "tip below height"
        );
        assert_eq!(
            timelock_spendable(&abs, 5, 900_000),
            Some(true),
            "tip reached height"
        );
        let time = PolicyTimelock {
            kind: "relative".into(),
            value: 512,
            blocks: false,
            label: String::new(),
        };
        assert_eq!(
            timelock_spendable(&time, 999, 1_000),
            None,
            "time-based → don't pre-gate"
        );
    }

    #[test]
    fn multipath_export_combines_keychains_for_vault() {
        let p = [signer(1, "m/48'/0'/0'/2'")];
        let r = [signer(2, "m/48'/0'/0'/2'")];
        let v = descriptor_from_inheritance_vault(
            1,
            &p,
            1,
            &r,
            VaultTimelock::RelativeBlocks(26280),
            Network::Bitcoin,
        )
        .unwrap();
        let mp = descriptor_to_multipath(&v.external).unwrap();
        assert!(mp.contains("/<0;1>/*"), "multipath form: {mp}");
        assert!(
            !mp.contains("/0/*"),
            "no single-chain derivation left: {mp}"
        );
        assert!(mp.contains('#'), "must carry a checksum: {mp}");
        // Taproot too.
        let tp = signer(3, "m/86'/0'/0'");
        let tr = [signer(4, "m/86'/0'/0'")];
        let tv = descriptor_from_taproot_vault(
            &tp,
            1,
            &tr,
            VaultTimelock::RelativeBlocks(100),
            Network::Bitcoin,
        )
        .unwrap();
        let tmp = descriptor_to_multipath(&tv.external).unwrap();
        assert!(
            tmp.starts_with("tr(") && tmp.contains("/<0;1>/*"),
            "taproot multipath: {tmp}"
        );
    }

    #[test]
    fn vault_external_sign_then_separate_finalize() {
        // Mirrors the hardware→broadcast path: produce partial sigs WITHOUT
        // finalizing (as an external signer does), then finalize separately via
        // the descriptor — exactly what /broadcast now does server-side.
        use bdk_wallet::bitcoin::{hashes::Hash, Amount, BlockHash, FeeRate, Network as N};
        use bdk_wallet::chain::BlockId;
        use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
        use bdk_wallet::{KeychainKind, SignOptions, Wallet};
        use std::collections::BTreeMap;

        let a = test_tprv(1);
        let b = test_tprv(2);
        let ext = format!("wsh(or_d(pk({a}/0/*),and_v(v:pk({b}/0/*),older(10))))");
        let int = format!("wsh(or_d(pk({a}/1/*),and_v(v:pk({b}/1/*),older(10))))");
        let mut w = Wallet::create(ext, int)
            .network(N::Regtest)
            .create_wallet_no_persist()
            .unwrap();
        insert_checkpoint(
            &mut w,
            BlockId {
                height: 1_000,
                hash: BlockHash::all_zeros(),
            },
        );
        let _ = receive_output_in_latest_block(&mut w, Amount::from_sat(100_000));
        let dest = w.reveal_next_address(KeychainKind::Internal).address;
        let ext_id = w.policies(KeychainKind::External).unwrap().unwrap().id;
        let int_id = w.policies(KeychainKind::Internal).unwrap().unwrap().id;

        // Recovery (script-path) spend.
        let mut builder = w.build_tx();
        builder.fee_rate(FeeRate::from_sat_per_vb(2).unwrap());
        builder.add_recipient(dest.script_pubkey(), Amount::from_sat(40_000));
        builder.policy_path(BTreeMap::from([(ext_id, vec![1])]), KeychainKind::External);
        builder.policy_path(BTreeMap::from([(int_id, vec![1])]), KeychainKind::Internal);
        builder.current_height(1_010);
        let mut psbt = builder.finish().unwrap();

        // Sign-only (no finalize), like an external/HW signer leaving partial sigs.
        let finalized = w
            .sign(
                &mut psbt,
                SignOptions {
                    try_finalize: false,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(
            !finalized,
            "sign-only must report the PSBT as not finalized"
        );
        assert!(
            psbt.inputs[0].final_script_witness.is_none(),
            "no witness assembled after sign-only"
        );

        // Separate finalize via the descriptor — what /broadcast does.
        let done = w.finalize_psbt(&mut psbt, SignOptions::default()).unwrap();
        assert!(
            done,
            "descriptor finalize must complete the script-path spend"
        );
        assert!(
            psbt.inputs[0].final_script_witness.is_some(),
            "witness assembled after finalize"
        );
        assert!(
            psbt.extract_tx().is_ok(),
            "finalized PSBT extracts a broadcastable tx"
        );
    }

    #[test]
    fn taproot_vault_spends_via_keypath_and_script_path() {
        // Same as the wsh test but for a taproot vault: confirms the
        // primary=keypath / recovery=tapleaf branch selection BDK exposes, and
        // that the spend_path→branch index mapping holds for taproot too.
        use bdk_wallet::bitcoin::{hashes::Hash, Amount, BlockHash, FeeRate, Network as N};
        use bdk_wallet::chain::BlockId;
        use bdk_wallet::test_utils::{insert_checkpoint, receive_output_in_latest_block};
        use bdk_wallet::{KeychainKind, SignOptions, Wallet};
        use std::collections::BTreeMap;

        let a = test_tprv(1);
        let b = test_tprv(2);
        let ext = format!("tr({a}/0/*,and_v(v:pk({b}/0/*),older(10)))");
        let int = format!("tr({a}/1/*,and_v(v:pk({b}/1/*),older(10)))");

        let try_branch = |branch: usize, height: u32| -> Result<bool, String> {
            let mut w = Wallet::create(ext.clone(), int.clone())
                .network(N::Regtest)
                .create_wallet_no_persist()
                .unwrap();
            insert_checkpoint(
                &mut w,
                BlockId {
                    height: 1_000,
                    hash: BlockHash::all_zeros(),
                },
            );
            let _ = receive_output_in_latest_block(&mut w, Amount::from_sat(100_000));
            let dest = w.reveal_next_address(KeychainKind::Internal).address;
            let ext_id = w.policies(KeychainKind::External).unwrap().unwrap().id;
            let int_id = w.policies(KeychainKind::Internal).unwrap().unwrap().id;
            let mut builder = w.build_tx();
            builder.fee_rate(FeeRate::from_sat_per_vb(2).unwrap());
            builder.add_recipient(dest.script_pubkey(), Amount::from_sat(40_000));
            builder.policy_path(
                BTreeMap::from([(ext_id, vec![branch])]),
                KeychainKind::External,
            );
            builder.policy_path(
                BTreeMap::from([(int_id, vec![branch])]),
                KeychainKind::Internal,
            );
            builder.current_height(height);
            let mut psbt = builder.finish().map_err(|e| format!("{e:?}"))?;
            w.sign(&mut psbt, SignOptions::default())
                .map_err(|e| format!("{e:?}"))
        };

        // Key-path (primary) now, and the timelocked tapleaf (recovery) at 1010.
        assert_eq!(
            try_branch(0, 1_000),
            Ok(true),
            "taproot key-path must finalize"
        );
        assert_eq!(
            try_branch(1, 1_010),
            Ok(true),
            "taproot script-path (recovery) must finalize"
        );
    }

    #[test]
    fn import_wrong_network_rejected_by_bdk() {
        // test_xpub(1) is mainnet. import_descriptor relies on BDK at
        // open_or_create_wallet to refuse a mismatched-network descriptor —
        // verify that reliance holds (create a wallet on Testnet, expect error).
        use bdk_wallet::Wallet;
        let (p, _) = parse_descriptor_import(&format!("wpkh({}/0/*)", test_xpub(1)), None).unwrap();
        let res = Wallet::create(p.external, p.internal.unwrap())
            .network(Network::Testnet)
            .create_wallet_no_persist();
        assert!(
            res.is_err(),
            "BDK must reject a mainnet descriptor on testnet"
        );
    }
}
