//! Bytewise ASCII-insensitive substring search with linear worst-case work.
//! The caller validates the 1,024-byte query before preprocessing. Non-ASCII
//! bytes compare exactly; this deliberately does not perform Unicode folding.

pub(super) struct AsciiQuery<'a> {
    needle: &'a [u8],
    prefixes: Vec<usize>,
    #[cfg(test)]
    comparisons: std::cell::Cell<usize>,
}

impl<'a> AsciiQuery<'a> {
    pub(super) fn new(needle: &'a [u8]) -> Self {
        let mut query = Self {
            needle,
            prefixes: vec![0; needle.len()],
            #[cfg(test)]
            comparisons: std::cell::Cell::new(0),
        };
        let mut matched = 0;
        for (index, &byte) in needle.iter().enumerate().skip(1) {
            loop {
                if query.equal(byte, matched) {
                    matched += 1;
                    break;
                }
                if matched == 0 {
                    break;
                }
                matched = query.prefixes[matched - 1];
            }
            query.prefixes[index] = matched;
        }
        query
    }

    pub(super) fn contains(&self, haystack: &[u8]) -> bool {
        if self.needle.is_empty() {
            return true;
        }
        if haystack.len() < self.needle.len() {
            return false;
        }
        let mut matched = 0;
        for &byte in haystack {
            loop {
                if self.equal(byte, matched) {
                    matched += 1;
                    if matched == self.needle.len() {
                        return true;
                    }
                    break;
                }
                if matched == 0 {
                    break;
                }
                // Each fallback reduces a prefix previously advanced by a
                // consumed byte: at most two comparisons per haystack byte.
                matched = self.prefixes[matched - 1];
            }
        }
        false
    }

    fn equal(&self, byte: u8, index: usize) -> bool {
        #[cfg(test)]
        self.comparisons.set(self.comparisons.get() + 1);
        byte.eq_ignore_ascii_case(&self.needle[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repetitive_maximum_query_work_is_linear_across_the_complete_scan() {
        let description = vec![b'a'; 4096];
        for length in [32, 256, 1024] {
            let mut needle = vec![b'A'; length];
            needle[length - 1] = b'b';
            let query = AsciiQuery::new(&needle);
            assert!(query.comparisons.get() <= 2 * needle.len());
            for _ in 0..480 {
                assert!(!query.contains(b"short name"));
                assert!(!query.contains(&description));
                assert!(!query.contains(b"/short/location"));
            }
            assert!(query.comparisons.get() <= 2 * (needle.len() + 480 * description.len()));
            let mut found = description.clone();
            found.push(b'B');
            let before = query.comparisons.get();
            assert!(query.contains(&found));
            assert!(query.comparisons.get() - before <= 2 * found.len());
        }
    }

    #[test]
    fn ascii_overlap_and_non_ascii_bytes_match_the_reference_semantics() {
        for (haystack, needle, expected) in [
            ("", "", true),
            ("a", "ab", false),
            ("aBaBaBaC", "ABAbAC", true),
            ("aBaBaBaC", "ABAbAD", false),
            ("review🦀É", "VIEW🦀É", true),
            ("review🦀É", "view🦀é", false),
            ("résumé", "RÉSUMÉ", false),
            ("a\0B", "\0b", true),
        ] {
            assert_eq!(
                AsciiQuery::new(needle.as_bytes()).contains(haystack.as_bytes()),
                expected
            );
        }
        // Exhaustive small byte strings exercise fallback after both matching
        // and mismatching prefixes, including ASCII aliases and opaque bytes.
        let alphabet = [b'a', b'A', b'b', 0x80];
        let strings = (0..=5)
            .flat_map(|length| {
                (0..4_usize.pow(length)).map(move |mut value| {
                    (0..length)
                        .map(|_| {
                            let byte = alphabet[value % 4];
                            value /= 4;
                            byte
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        for needle in strings.iter().filter(|text| text.len() <= 3) {
            let query = AsciiQuery::new(needle);
            for haystack in &strings {
                let expected = needle.is_empty()
                    || haystack
                        .windows(needle.len())
                        .any(|window| window.eq_ignore_ascii_case(needle));
                assert_eq!(
                    query.contains(haystack),
                    expected,
                    "{haystack:?} / {needle:?}"
                );
            }
        }
    }
}
