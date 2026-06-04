#![no_main]
//! Fuzz the main untrusted-input entry point: `parse_input` accepts arbitrary
//! user-pasted strings (descriptors, xpubs, addresses) and must never panic,
//! hang, or overflow — only ever return Ok/Err.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = corvin_core::descriptor::parse_input(s, bitcoin::Network::Bitcoin);
        let _ = corvin_core::descriptor::parse_input(s, bitcoin::Network::Testnet);
    }
});
