//! Markdown → ADF → Markdown → ADF: both ADF documents are valid, and the round-trip guarantee
//! from `docs/syntax.md` holds (`md_to_adf(adf_to_md(x)) == normalize(x)`).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((options, markdown)) = adf_fuzz::split_input(data) else {
        return;
    };
    if let Some(doc) = adf_fuzz::check_md_to_adf(&markdown, &options) {
        adf_fuzz::check_round_trip(&doc, &options);
    }
});
