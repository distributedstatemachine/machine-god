use super::{
    MAX_NATIVE_SKILL_DESCRIPTION_BYTES, MAX_NATIVE_SKILL_HEADER_BYTES,
    MAX_NATIVE_SKILL_METADATA_NAME_BYTES, NativeSkillMetadata, NativeSkillMetadataError as Error,
    closing_delimiter, header_start,
};

type Result<T> = std::result::Result<T, Error>;

pub(super) fn parse(bytes: &[u8], fallback: &str) -> Result<NativeSkillMetadata> {
    let Some(start) = header_start(bytes) else {
        validate_name(fallback.as_bytes())?;
        return Ok(NativeSkillMetadata {
            name: fallback.to_owned(),
            description: String::new(),
            body_offset: 0,
            has_frontmatter: false,
        });
    };
    let (end, body_offset) =
        closing_delimiter(bytes, start).ok_or(if bytes.len() > MAX_NATIVE_SKILL_HEADER_BYTES {
            Error::HeaderTooLong
        } else {
            Error::MissingClosingDelimiter
        })?;
    let mut fields = Fields::default();
    let header = &bytes[start..end];
    let mut offset = 0;
    while let Some((line, next)) = line_at(header, offset) {
        offset = next;
        fields.line(header, line, &mut offset);
    }
    fields.finish(body_offset)
}

#[derive(Default)]
struct Fields {
    name: Option<Vec<u8>>,
    description: Option<String>,
    saw_description: bool,
    previous_recognized: bool,
    error: Option<Error>,
}

impl Fields {
    fn line(&mut self, header: &[u8], line: &[u8], offset: &mut usize) {
        let trimmed = trim(line);
        if trimmed.is_empty() || trimmed.starts_with(b"#") {
            return;
        }
        if self.previous_recognized && matches!(line.first(), Some(b' ' | b'\t')) {
            self.error.get_or_insert(Error::UnsupportedMultiline);
            self.previous_recognized = false;
            return;
        }
        self.previous_recognized = false;
        let Some(colon) = trimmed.iter().position(|byte| *byte == b':') else {
            return;
        };
        let key = trim(&trimmed[..colon]);
        let raw = trim(&trimmed[colon + 1..]);
        match key {
            b"name" => {
                self.previous_recognized = true;
                if self.name.is_some() {
                    self.error.get_or_insert(Error::DuplicateRecognizedKey);
                }
                self.name = Some(self.record(value(raw)).unwrap_or(raw).to_vec());
            }
            b"description" => {
                if self.saw_description {
                    self.error.get_or_insert(Error::DuplicateRecognizedKey);
                }
                self.saw_description = true;
                if matches!(raw, b">" | b">-" | b"|") {
                    self.description = self.record(block(header, offset, raw));
                } else {
                    self.previous_recognized = true;
                    self.description = self.record(
                        value(raw).and_then(|bytes| bounded_description(bytes).map(str::to_owned)),
                    );
                }
            }
            _ => {}
        }
    }

    fn record<T>(&mut self, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(error) => {
                self.error.get_or_insert(error);
                None
            }
        }
    }

    fn finish(mut self, body_offset: usize) -> Result<NativeSkillMetadata> {
        let name = self.name.take().unwrap_or_default();
        let validated = self.record(validate_name(&name));
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(NativeSkillMetadata {
            name: validated.ok_or(Error::MissingName)?.to_owned(),
            description: self.description.unwrap_or_default(),
            body_offset,
            has_frontmatter: true,
        })
    }
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'))
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

fn line_at(bytes: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let tail = bytes.get(offset..).filter(|tail| !tail.is_empty())?;
    let count = tail.iter().position(|byte| *byte == b'\n');
    let end = count.unwrap_or(tail.len());
    let raw = &tail[..end];
    Some((
        raw.strip_suffix(b"\r").unwrap_or(raw),
        offset + end + usize::from(count.is_some()),
    ))
}

fn value(bytes: &[u8]) -> Result<&[u8]> {
    if matches!(bytes.first(), Some(b'|' | b'>')) {
        return Err(Error::UnsupportedMultiline);
    }
    let quoted = |byte: Option<&u8>| matches!(byte, Some(b'\'' | b'"'));
    if quoted(bytes.first()) || quoted(bytes.last()) {
        if bytes.len() < 2 || bytes.first() != bytes.last() {
            return Err(Error::MalformedQuote);
        }
        return Ok(&bytes[1..bytes.len() - 1]);
    }
    Ok(bytes)
}

fn text(bytes: &[u8]) -> Result<&str> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
    if bytes.iter().any(|byte| *byte < 0x20 || *byte == 0x7f) {
        return Err(Error::ControlByte);
    }
    Ok(text)
}

fn validate_name(bytes: &[u8]) -> Result<&str> {
    if bytes.is_empty() {
        return Err(Error::MissingName);
    }
    if bytes.len() > MAX_NATIVE_SKILL_METADATA_NAME_BYTES {
        return Err(Error::NameTooLong);
    }
    let name = text(bytes)?;
    if matches!(name, "." | "..") || name.contains(['/', '\\']) {
        return Err(Error::InvalidName);
    }
    Ok(name)
}

fn bounded_description(bytes: &[u8]) -> Result<&str> {
    if bytes.len() > MAX_NATIVE_SKILL_DESCRIPTION_BYTES {
        return Err(Error::DescriptionTooLong);
    }
    text(bytes)
}

fn block(header: &[u8], offset: &mut usize, style: &[u8]) -> Result<String> {
    let mut lines = Vec::new();
    let mut indent = None;
    while let Some((line, next)) = line_at(header, *offset) {
        if trim(line).is_empty() {
            lines.push(&b""[..]);
        } else {
            let spaces = line.iter().take_while(|byte| **byte == b' ').count();
            if line.get(spaces) == Some(&b'\t') {
                return Err(Error::UnsupportedMultiline);
            }
            if spaces == 0 {
                break;
            }
            let base = *indent.get_or_insert(spaces);
            if spaces < base {
                return Err(Error::UnsupportedMultiline);
            }
            text(&line[base..])?;
            lines.push(&line[base..]);
        }
        *offset = next;
    }
    while lines.last() == Some(&&b""[..]) {
        lines.pop();
    }
    let mut output = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            let folded = style != b"|" && !line.is_empty() && !lines[index - 1].is_empty();
            output.push(if folded { ' ' } else { '\n' });
        }
        output.push_str(std::str::from_utf8(line).map_err(|_| Error::InvalidUtf8)?);
        if output.len() > MAX_NATIVE_SKILL_DESCRIPTION_BYTES {
            return Err(Error::DescriptionTooLong);
        }
    }
    if !lines.is_empty() && style != b">-" {
        output.push('\n');
    }
    if output.len() > MAX_NATIVE_SKILL_DESCRIPTION_BYTES {
        return Err(Error::DescriptionTooLong);
    }
    Ok(output)
}
