//! Allocation-free UTF-8 boundary inspection shared by portable terminal code.

pub(crate) fn incomplete_utf8_suffix_len(bytes: &[u8]) -> usize {
    let continuation_bytes = bytes
        .iter()
        .rev()
        .take(3)
        .take_while(|byte| matches!(byte, 0x80..=0xbf))
        .count();
    let Some(lead_index) = bytes.len().checked_sub(continuation_bytes + 1) else {
        return 0;
    };
    let lead = bytes[lead_index];
    let expected_length = match lead {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 0,
    };
    let available_length = continuation_bytes + 1;
    if available_length >= expected_length
        || !partial_utf8_scalar_is_valid(&bytes[lead_index..], expected_length)
    {
        0
    } else {
        available_length
    }
}

fn partial_utf8_scalar_is_valid(bytes: &[u8], expected_length: usize) -> bool {
    if bytes.len() >= 2 {
        let second = bytes[1];
        let valid_second = match bytes[0] {
            0xe0 => matches!(second, 0xa0..=0xbf),
            0xed => matches!(second, 0x80..=0x9f),
            0xf0 => matches!(second, 0x90..=0xbf),
            0xf4 => matches!(second, 0x80..=0x8f),
            _ => matches!(second, 0x80..=0xbf),
        };
        if !valid_second {
            return false;
        }
    }
    bytes
        .iter()
        .skip(2)
        .take(expected_length.saturating_sub(2))
        .all(|byte| matches!(byte, 0x80..=0xbf))
}
