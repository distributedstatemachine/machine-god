// SPDX-License-Identifier: Apache-2.0
//
// Safe Rust transliteration of vercel-labs/fx at revision
// b1774fbf6c7602b503026f96f6e960e946c692ef:
// src/core/shared/display_width.zig. Unicode lookup data is generated from the
// pinned sibling source by scripts/generate_terminal_unicode_data.py.

use super::terminal_unicode_data::{
    EMOJI_MODIFIER_RANGES, EMOJI_PRESENTATION_RANGES, MAX_RGI_SEQUENCE_CODEPOINTS, RGI_TRIE_EDGES,
    RGI_TRIE_NODES, Range, VARIATION_BASES, WIDE_RANGES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DecodedRune {
    pub(super) len: usize,
    pub(super) codepoint: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DisplayUnit {
    pub(super) byte_len: usize,
    pub(super) cell_width: u8,
}

pub(super) fn display_unit_at(text: &[u8], index: usize) -> DisplayUnit {
    let Some(&first_byte) = text.get(index) else {
        return DisplayUnit {
            byte_len: 0,
            cell_width: 0,
        };
    };
    let may_start_keycap =
        matches!(first_byte, b'#' | b'*' | b'0'..=b'9') && has_rgi_keycap_suffix(text, index + 1);
    if first_byte < 0x80 && !may_start_keycap {
        return DisplayUnit {
            byte_len: 1,
            cell_width: u8::from(first_byte >= 32 && first_byte != 0x7f),
        };
    }

    let rgi_len = match_rgi_sequence(text, index);
    if rgi_len != 0 {
        return DisplayUnit {
            byte_len: rgi_len,
            cell_width: 2,
        };
    }

    let first = decode_next_rune(text, index);
    let next_index = index + first.len;
    if next_index < text.len() && in_ranges(first.codepoint, &VARIATION_BASES) {
        let selector = decode_next_rune(text, next_index);
        if selector.codepoint == 0xfe0e {
            return DisplayUnit {
                byte_len: first.len + selector.len,
                cell_width: if in_ranges(first.codepoint, &WIDE_RANGES) {
                    2
                } else {
                    1
                },
            };
        }
        if selector.codepoint == 0xfe0f {
            return DisplayUnit {
                byte_len: first.len + selector.len,
                cell_width: 2,
            };
        }
    }

    DisplayUnit {
        byte_len: first.len,
        cell_width: rune_width(first.codepoint),
    }
}

fn has_rgi_keycap_suffix(text: &[u8], suffix_start: usize) -> bool {
    let Some(mut suffix) = text.get(suffix_start..) else {
        return false;
    };
    if suffix.starts_with("\u{fe0f}".as_bytes()) {
        suffix = &suffix['\u{fe0f}'.len_utf8()..];
    }
    suffix.starts_with("\u{20e3}".as_bytes())
}

pub(super) fn decode_next_rune(text: &[u8], index: usize) -> DecodedRune {
    let Some(&first) = text.get(index) else {
        return DecodedRune {
            len: 0,
            codepoint: 0,
        };
    };
    if first < 0x80 {
        return DecodedRune {
            len: 1,
            codepoint: u32::from(first),
        };
    }
    let Some(len) = utf8_sequence_len(first) else {
        return replacement();
    };
    let Some(candidate) = text.get(index..index.saturating_add(len)) else {
        return replacement();
    };
    let Ok(value) = std::str::from_utf8(candidate) else {
        return replacement();
    };
    let Some(ch) = value.chars().next() else {
        return replacement();
    };
    DecodedRune {
        len,
        codepoint: u32::from(ch),
    }
}

pub(super) fn utf8_sequence_len(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        // Pinned Zig classifies all `11110xxx` prefixes as four-byte
        // sequences here. Scalar validity is checked separately by the
        // decoder after the complete sequence has been collected.
        0xf0..=0xf7 => Some(4),
        _ => None,
    }
}

fn replacement() -> DecodedRune {
    DecodedRune {
        len: 1,
        codepoint: 0xfffd,
    }
}

fn rune_width(codepoint: u32) -> u8 {
    if codepoint == 0 || codepoint < 32 || (0x7f..0xa0).contains(&codepoint) {
        return 0;
    }
    if is_zero_width_continuation(codepoint) {
        return 0;
    }
    if in_ranges(codepoint, &WIDE_RANGES) || in_ranges(codepoint, &EMOJI_PRESENTATION_RANGES) {
        return 2;
    }
    1
}

fn is_zero_width_continuation(codepoint: u32) -> bool {
    is_combining(codepoint)
        || in_ranges(codepoint, &EMOJI_MODIFIER_RANGES)
        || codepoint == 0x200c
        || codepoint == 0x200d
        || (0x200b..=0x200f).contains(&codepoint)
        || (0x202a..=0x202e).contains(&codepoint)
        || (0x2060..=0x206f).contains(&codepoint)
        || codepoint == 0xfeff
        || (0xe0001..=0xe007f).contains(&codepoint)
        || (0xe0100..=0xe01ef).contains(&codepoint)
}

fn is_combining(codepoint: u32) -> bool {
    (0x0300..=0x036f).contains(&codepoint)
        || (0x1ab0..=0x1aff).contains(&codepoint)
        || (0x1dc0..=0x1dff).contains(&codepoint)
        || (0x20d0..=0x20ff).contains(&codepoint)
        || (0xfe20..=0xfe2f).contains(&codepoint)
        || (0xfe00..=0xfe0f).contains(&codepoint)
}

fn in_ranges(codepoint: u32, ranges: &[Range]) -> bool {
    ranges
        .binary_search_by(|range| {
            if codepoint < range.first {
                std::cmp::Ordering::Greater
            } else if codepoint > range.last {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn match_rgi_sequence(text: &[u8], index: usize) -> usize {
    let mut node_index = 0_u32;
    let mut cursor = index;
    let mut longest = 0;
    for _ in 0..MAX_RGI_SEQUENCE_CODEPOINTS {
        if cursor >= text.len() {
            break;
        }
        let rune = decode_next_rune(text, cursor);
        let Some(child) = find_trie_child(node_index, rune.codepoint) else {
            break;
        };
        node_index = child;
        cursor += rune.len;
        if RGI_TRIE_NODES[node_index as usize].terminal {
            longest = cursor - index;
        }
    }
    longest
}

fn find_trie_child(node_index: u32, codepoint: u32) -> Option<u32> {
    let node = RGI_TRIE_NODES.get(node_index as usize)?;
    let start = node.edge_start as usize;
    let end = start.checked_add(node.edge_len as usize)?;
    let edges = RGI_TRIE_EDGES.get(start..end)?;
    edges
        .binary_search_by_key(&codepoint, |edge| edge.codepoint)
        .ok()
        .map(|index| edges[index].child)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_unicode_17_width_policy_and_sequence_spans() {
        let cases = [
            ("A", 1, 1),
            ("界", "界".len(), 2),
            ("✅", "✅".len(), 2),
            ("☀\u{fe0e}", "☀\u{fe0e}".len(), 1),
            ("⌚\u{fe0e}", "⌚\u{fe0e}".len(), 2),
            ("☀\u{fe0f}", "☀\u{fe0f}".len(), 2),
            ("👍🏽", "👍🏽".len(), 2),
            ("🇺🇸", "🇺🇸".len(), 2),
            ("#️⃣", "#️⃣".len(), 2),
            (
                "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}",
                "🏴\u{e0067}\u{e0062}\u{e0065}\u{e006e}\u{e0067}\u{e007f}".len(),
                2,
            ),
            ("👩‍💻", "👩‍💻".len(), 2),
        ];
        for (text, byte_len, width) in cases {
            assert_eq!(
                display_unit_at(text.as_bytes(), 0),
                DisplayUnit {
                    byte_len,
                    cell_width: width,
                }
            );
        }
        assert_eq!(
            display_unit_at("a\u{301}".as_bytes(), 0),
            DisplayUnit {
                byte_len: 1,
                cell_width: 1,
            }
        );
        assert_eq!(
            display_unit_at("a\u{301}".as_bytes(), 1),
            DisplayUnit {
                byte_len: "\u{301}".len(),
                cell_width: 0,
            }
        );
    }

    #[test]
    fn invalid_utf8_advances_one_byte_as_replacement() {
        assert_eq!(
            decode_next_rune(&[0xf0, 0x28, 0x8c, 0x28], 0),
            DecodedRune {
                len: 1,
                codepoint: 0xfffd,
            }
        );
    }

    #[test]
    fn non_scalar_four_byte_prefixes_keep_the_pinned_sequence_length() {
        for prefix in 0xf5..=0xf7 {
            assert_eq!(utf8_sequence_len(prefix), Some(4));
            assert_eq!(
                decode_next_rune(&[prefix, 0x80, 0x80, 0x80], 0),
                DecodedRune {
                    len: 1,
                    codepoint: 0xfffd,
                }
            );
        }
    }
}
