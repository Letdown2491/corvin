#![no_main]
//! Fuzz `describe_policy`, which parses an arbitrary descriptor string into a
//! human-readable policy summary. Must never panic on hostile input.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = corvin_core::descriptor::describe_policy(s);
    }
});
