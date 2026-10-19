//! Markdown → ADF on arbitrary bytes: never panics, never overflows the stack, and returns valid
//! ADF or a documented error. The first byte selects the options.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some((options, markdown)) = adf_fuzz::split_input(data) {
        let _ = adf_fuzz::check_md_to_adf(&markdown, &options);
    }
});
