//! Deterministic `localId` generation.
//!
//! Jira rejects `taskList`/`taskItem` nodes without a `localId`. Random IDs would make output
//! non-deterministic, so IDs are derived from a hash of the input plus an ordinal: the same input
//! always produces the same IDs, and different inputs produce different ones. IDs are formatted
//! as RFC 9562 version-8 ("custom") UUIDs, the most conservative format Jira accepts.

/// Generates unique, deterministic local IDs for one document.
#[derive(Debug, Clone)]
pub struct LocalIdGen {
    seed: u64,
    counter: u64,
}

impl LocalIdGen {
    /// Seeds the generator from the source text.
    pub fn new(source: &str) -> Self {
        Self {
            seed: fnv1a(source.as_bytes()),
            counter: 0,
        }
    }

    /// The next ID, e.g. `3f2a9c1e-7b4d-8e21-9a0f-1c2d3e4f5a6b`.
    pub fn next_id(&mut self) -> String {
        self.counter += 1;
        let hi = splitmix64(self.seed ^ self.counter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let lo = splitmix64(hi ^ self.counter);
        // Version 8 in the high nibble of byte 6, variant 0b10 in the top bits of byte 8.
        let hi = (hi & 0xFFFF_FFFF_FFFF_0FFF) | 0x0000_0000_0000_8000;
        let lo = (lo & 0x3FFF_FFFF_FFFF_FFFF) | 0x8000_0000_0000_0000;
        format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            hi >> 32,
            (hi >> 16) & 0xFFFF,
            hi & 0xFFFF,
            lo >> 48,
            lo & 0xFFFF_FFFF_FFFF
        )
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_unique_uuid_v8() {
        let mut a = LocalIdGen::new("- [ ] x");
        let mut b = LocalIdGen::new("- [ ] x");
        let mut c = LocalIdGen::new("- [ ] y");
        let ids: Vec<String> = (0..100).map(|_| a.next_id()).collect();
        assert_eq!(ids, (0..100).map(|_| b.next_id()).collect::<Vec<_>>());
        assert_ne!(ids[0], c.next_id());
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
        for id in &ids {
            assert_eq!(id.len(), 36);
            assert_eq!(&id[14..15], "8", "{id}");
            assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"), "{id}");
        }
    }
}
