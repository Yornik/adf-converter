//! ADF → Markdown on arbitrary JSON (bytes → `serde_json::Value` → `adf_to_md`), in both
//! flavours: never panics, fails only with input errors, and its Markdown converts back.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() > adf_fuzz::MAX_INPUT {
        return;
    }
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        adf_fuzz::check_adf_to_md(&value);
    }
});
