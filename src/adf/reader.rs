//! Lenient reading of arbitrary ADF for ADF → Markdown. Real documents contain `localId`s, attributes
//! and node types this crate never emits; nothing here may fail on them.
//!
//! OWNER: stream S2. Define the API you need; it is internal to `to_md`.
