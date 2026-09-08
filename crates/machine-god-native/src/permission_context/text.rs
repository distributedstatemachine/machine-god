use super::{Error, MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES};
use std::fmt::Write as _;
const CURRENT: &str = "current_request: ";
const FIRST: &str = "first_root_user_request: ";
const RECENT: &str = "recent_root_user_request: ";
const OMITTED: &str = "omitted_proven_root_user_turns: ";
const MARKER: &str = " [... omitted ...] ";
const CAP: usize = 1024;

fn encode(raw: &str) -> Result<String, Error> {
    let masked = crate::permission_reviewer::secrets::mask_root_text(
        raw,
        MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES,
    )
    .ok_or(Error::Limit)?;
    let mut out = String::new();
    for c in masked.chars() {
        let code = u32::from(c);
        if code <= 0x1f || code == 0x7f {
            write!(out, "\\x{code:02x}").map_err(|_| Error::Limit)?;
        } else if (0x80..=0x9f).contains(&code)
            || (0x200b..=0x200f).contains(&code)
            || (0x2028..=0x202e).contains(&code)
            || (0x2060..=0x206f).contains(&code)
            || code == 0xfeff
        {
            write!(out, "\\u{{{code:04x}}}").map_err(|_| Error::Limit)?;
        } else {
            out.push(c);
        }
    }
    Ok(out)
}
fn head_tail(out: &mut String, text: &str, cap: usize) {
    if text.len() <= cap {
        out.push_str(text);
        return;
    }
    if cap <= MARKER.len() {
        out.push_str(&MARKER[..cap]);
        return;
    }
    let retained = cap - MARKER.len();
    let mut head = retained.div_ceil(2);
    let mut tail = text.len() - (retained - head);
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    out.push_str(&text[..head]);
    out.push_str(MARKER);
    out.push_str(&text[tail..]);
}
fn budgets(lengths: [Option<usize>; 3], capacity: usize) -> [usize; 3] {
    let share = capacity / lengths.iter().flatten().count();
    let mut result = lengths.map(|len| len.unwrap_or(0).min(share));
    let mut remaining = capacity - result.iter().sum::<usize>();
    while remaining > 0 {
        let count = lengths
            .iter()
            .zip(result)
            .filter(|(len, budget)| len.is_some_and(|len| len > *budget))
            .count();
        if count == 0 {
            break;
        }
        let share = (remaining / count).max(1);
        for (len, budget) in lengths.iter().zip(&mut result) {
            let Some(len) = len else {
                continue;
            };
            let added = (len - *budget).min(share).min(remaining);
            *budget += added;
            remaining -= added;
            if remaining == 0 {
                break;
            }
        }
    }
    result
}
fn line(out: &mut String, label: &str, text: &str, cap: usize) {
    out.push_str(label);
    head_tail(out, text, cap);
    out.push('\n');
}
pub(super) fn project(
    current: &str,
    first: Option<&str>,
    recent: &[&str],
    prior_omitted: u64,
) -> Result<String, Error> {
    let current = encode(current)?;
    let first = first.map(encode).transpose()?;
    let newest = recent.first().map(|text| encode(text)).transpose()?;
    let omitted_after_required = prior_omitted
        .checked_add(recent.len().saturating_sub(1) as u64)
        .ok_or(Error::Limit)?;
    let overhead = CURRENT.len()
        + 1
        + first.as_ref().map_or(0, |_| FIRST.len() + 1)
        + newest.as_ref().map_or(0, |_| RECENT.len() + 1);
    let available = CAP - overhead - if omitted_after_required > 0 { 64 } else { 0 };
    let limits = budgets(
        [
            Some(current.len()),
            first.as_ref().map(String::len),
            newest.as_ref().map(String::len),
        ],
        available,
    );
    let mut out = String::with_capacity(CAP);
    line(&mut out, CURRENT, &current, limits[0]);
    if let Some(first) = first {
        line(&mut out, FIRST, &first, limits[1]);
    }
    if let Some(newest) = newest {
        line(&mut out, RECENT, &newest, limits[2]);
    }
    let mut selected = usize::from(!recent.is_empty());
    for (offset, older) in recent.iter().enumerate().skip(1) {
        let reserve = if offset + 1 < recent.len() || prior_omitted > 0 {
            64
        } else {
            0
        };
        let overhead = RECENT.len() + 1;
        if out.len() + overhead + MARKER.len() + reserve > CAP {
            break;
        }
        let encoded = encode(older)?;
        let available = CAP - out.len() - overhead - reserve;
        line(&mut out, RECENT, &encoded, available);
        selected += 1;
    }
    let omitted = prior_omitted
        .checked_add((recent.len() - selected) as u64)
        .ok_or(Error::Limit)?;
    if omitted > 0 {
        writeln!(out, "{OMITTED}{omitted}").map_err(|_| Error::Limit)?;
    }
    if out.len() > CAP {
        return Err(Error::Limit);
    }
    crate::NativeAutoPermissionRootContext::from_proven_projection(&out)
        .map_err(|_| Error::InvalidProvenance)?;
    Ok(out)
}
