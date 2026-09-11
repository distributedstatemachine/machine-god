//! Pinned one-variable URI-template grammar, without matching or network effects.
pub(super) fn valid(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut literal = 0;
    let mut expressions = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'}' => return false,
            b'{' => {
                if expressions > 0 && literal == cursor
                    || !valid_literal(&bytes[literal..cursor])
                    || expressions == 64
                {
                    return false;
                }
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor] != b'}' {
                    if bytes[cursor] == b'{' {
                        return false;
                    }
                    cursor += 1;
                }
                if cursor == bytes.len() || start == cursor {
                    return false;
                }
                let mut variable = &bytes[start..cursor];
                if b"+#./;?&".contains(&variable[0]) {
                    variable = &variable[1..];
                }
                if variable.is_empty()
                    || variable.starts_with(b".")
                    || variable.ends_with(b".")
                    || variable.windows(2).any(|pair| pair == b"..")
                    || !variable
                        .iter()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_.".contains(byte))
                {
                    return false;
                }
                expressions += 1;
                cursor += 1;
                literal = cursor;
            }
            _ => cursor += 1,
        }
    }
    valid_literal(&bytes[literal..])
}
fn valid_literal(bytes: &[u8]) -> bool {
    let mut cursor = 0;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte == b'%' {
            if cursor + 2 >= bytes.len()
                || !bytes[cursor + 1].is_ascii_hexdigit()
                || !bytes[cursor + 2].is_ascii_hexdigit()
            {
                return false;
            }
            cursor += 3;
        } else {
            if !byte.is_ascii_alphanumeric() && !b"-._~:/?#[]@!$&'()*+,;=".contains(&byte) {
                return false;
            }
            cursor += 1;
        }
    }
    true
}
