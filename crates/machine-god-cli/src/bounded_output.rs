//! Byte-bounded staging for command output; commands own rendering and errors.

#[derive(Debug)]
pub(crate) struct BoundedOutput {
    value: String,
    max_bytes: usize,
}

impl BoundedOutput {
    pub(crate) fn with_capacity(max_bytes: usize, initial_capacity: usize) -> Self {
        Self {
            value: String::with_capacity(initial_capacity.min(max_bytes)),
            max_bytes,
        }
    }

    pub(crate) fn finish(self) -> String {
        self.value
    }

    pub(crate) fn len(&self) -> usize {
        self.value.len()
    }
}

impl std::fmt::Write for BoundedOutput {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let Some(new_len) = self.len().checked_add(value.len()) else {
            return Err(std::fmt::Error);
        };
        if new_len > self.max_bytes {
            return Err(std::fmt::Error);
        }
        self.value.push_str(value);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedOutput;
    use std::fmt::Write as _;

    #[test]
    fn empty_output_and_zero_limit_accept_only_empty_writes() {
        let mut output = BoundedOutput::with_capacity(0, 1024);
        assert_eq!(output.value.capacity(), 0);
        output.write_str("").unwrap();
        assert!(output.write_char('x').is_err());
        assert_eq!(output.finish(), "");
    }

    #[test]
    fn exact_limit_is_inclusive_and_failed_append_preserves_prefix() {
        let mut output = BoundedOutput::with_capacity(4, 2);
        output.write_str("ab").unwrap();
        assert!(output.write_str("cde").is_err());
        assert_eq!(output.value, "ab");
        output.write_str("cd").unwrap();
        assert!(output.write_char('e').is_err());
        output.write_str("").unwrap();
        assert_eq!(output.finish(), "abcd");
    }

    #[test]
    fn multibyte_characters_count_bytes_without_partial_append() {
        let mut output = BoundedOutput::with_capacity(7, 0);
        output.write_char('é').unwrap();
        assert!(output.write_str("日本").is_err());
        output.write_char('日').unwrap();
        output.write_char('é').unwrap();
        assert!(output.write_char('x').is_err());
        assert_eq!(output.finish(), "é日é");
    }

    #[test]
    fn formatted_append_obeys_the_same_byte_ceiling() {
        let mut output = BoundedOutput::with_capacity(5, 1);
        write!(output, "{}:{}", 12, 34).unwrap();
        assert!(writeln!(output).is_err());
        assert_eq!(output.finish(), "12:34");
    }

    #[test]
    fn capacity_is_a_hint_and_does_not_increase_the_limit() {
        let mut output = BoundedOutput::with_capacity(3, 8192);
        output.write_str("abc").unwrap();
        assert!(output.write_str("d").is_err());
        assert_eq!(output.finish(), "abc");
    }
}
