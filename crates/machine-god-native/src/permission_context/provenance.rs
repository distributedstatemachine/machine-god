use super::{Error, MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES, NATIVE_PERMISSION_CONTEXT_KEY, text};
use machine_god_core::{ContentBlock, Role, SessionRecord};
use serde_json::{Value, json};

const MAX_RECENT: usize = 32;
struct Provenance {
    current: usize,
    first: Option<usize>,
    recent: Vec<usize>,
    omitted: u64,
}

fn index(value: &Value) -> Result<usize, Error> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(Error::InvalidProvenance)
}
fn root_text(record: &SessionRecord, index: usize) -> Result<&str, Error> {
    let message = record.messages.get(index).ok_or(Error::InvalidProvenance)?;
    let [ContentBlock::Text { text }] = message.content.as_slice() else {
        return Err(Error::InvalidProvenance);
    };
    if message.role != Role::User || text.is_empty() {
        return Err(Error::InvalidProvenance);
    }
    if text.len() > MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES {
        return Err(Error::Limit);
    }
    Ok(text)
}
fn decode(record: &SessionRecord) -> Result<Option<Provenance>, Error> {
    let Some(value) = record.metadata.get(NATIVE_PERMISSION_CONTEXT_KEY) else {
        return Ok(None);
    };
    let value = value.as_object().ok_or(Error::InvalidProvenance)?;
    if value.len() != 6
        || value.get("version").and_then(Value::as_u64) != Some(1)
        || value.get("incarnation").and_then(Value::as_str) != Some(record.incarnation_id.as_str())
    {
        return Err(Error::InvalidProvenance);
    }
    let current = index(value.get("current").ok_or(Error::InvalidProvenance)?)?;
    let first = match value.get("first").ok_or(Error::InvalidProvenance)? {
        Value::Null => None,
        value => Some(index(value)?),
    };
    let recent = value
        .get("recent")
        .and_then(Value::as_array)
        .ok_or(Error::InvalidProvenance)?;
    if recent.len() > MAX_RECENT {
        return Err(Error::Limit);
    }
    let omitted = value
        .get("omitted")
        .and_then(Value::as_u64)
        .ok_or(Error::InvalidProvenance)?;
    root_text(record, current)?;
    if let Some(first) = first {
        if first > current {
            return Err(Error::InvalidProvenance);
        }
        root_text(record, first)?;
    }
    let mut indices = Vec::with_capacity(recent.len());
    let mut previous = current;
    for value in recent {
        let next = index(value)?;
        if next >= previous || first.is_some_and(|first| next <= first) {
            return Err(Error::InvalidProvenance);
        }
        root_text(record, next)?;
        indices.push(next);
        previous = next;
    }
    Ok(Some(Provenance {
        current,
        first,
        recent: indices,
        omitted,
    }))
}
pub(super) fn validate(record: &SessionRecord) -> Result<(), Error> {
    decode(record).map(|_| ())
}

pub(super) fn prepare(
    record: &mut SessionRecord,
    prompt: Option<&str>,
    current_user: usize,
) -> Result<Option<String>, Error> {
    let old = decode(record)?;
    let Some(prompt) = prompt else {
        let Some(old) = old.filter(|old| old.current == current_user) else {
            return Ok(None);
        };
        return format(record, &old, root_text(record, old.current)?).map(Some);
    };
    if prompt.is_empty() || prompt.len() > MAX_NATIVE_PERMISSION_ROOT_REQUEST_BYTES {
        return Err(Error::Limit);
    }
    if current_user != record.messages.len() {
        return Err(Error::InvalidProvenance);
    }
    let mut new = if let Some(mut old) = old {
        if old.first != Some(old.current) {
            old.recent.insert(0, old.current);
        }
        if old.recent.len() > MAX_RECENT {
            old.recent.pop();
            old.omitted = old.omitted.checked_add(1).ok_or(Error::Limit)?;
        }
        old.current = current_user;
        old
    } else {
        Provenance {
            current: current_user,
            first: record.messages.is_empty().then_some(current_user),
            recent: vec![],
            omitted: 0,
        }
    };
    // First/current may coincide only on the actual first native admission.
    if new.first == Some(current_user) {
        new.recent.clear();
    }
    let projection = format(record, &new, prompt)?;
    record.metadata.insert(
        NATIVE_PERMISSION_CONTEXT_KEY.to_owned(),
        json!({"version":1,
        "incarnation":record.incarnation_id.as_str(),"current":new.current,"first":new.first,
        "recent":new.recent,"omitted":new.omitted}),
    );
    Ok(Some(projection))
}

fn format(record: &SessionRecord, state: &Provenance, current: &str) -> Result<String, Error> {
    let first = state
        .first
        .filter(|first| *first != state.current)
        .map(|i| root_text(record, i))
        .transpose()?;
    let recent = state
        .recent
        .iter()
        .map(|i| root_text(record, *i))
        .collect::<Result<Vec<_>, _>>()?;
    text::project(current, first, &recent, state.omitted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use machine_god_core::{Message, SessionId, SessionIncarnationId};
    fn record() -> SessionRecord {
        SessionRecord::empty(
            SessionId::new("roots").unwrap(),
            SessionIncarnationId::new("life").unwrap(),
        )
    }
    #[test]
    fn bounded_provenance_keeps_first_current_recent_and_exact_omissions() {
        let mut record = record();
        let mut projection = String::new();
        for i in 0..100 {
            let prompt = format!("root {i}");
            let index = record.messages.len();
            projection = prepare(&mut record, Some(&prompt), index).unwrap().unwrap();
            record.messages.push(Message::text(Role::User, prompt));
            validate(&record).unwrap();
            assert!(projection.len() <= 1024);
        }
        let state = decode(&record).unwrap().unwrap();
        assert_eq!(state.first, Some(0));
        assert_eq!(state.current, 99);
        assert_eq!(state.recent.len(), 32);
        assert_eq!(state.omitted, 66);
        assert!(projection.contains("first_root_user_request: root 0"));
        assert!(projection.contains("recent_root_user_request: root 98"));
        assert!(projection.contains("omitted_proven_root_user_turns:"));
        assert_eq!(
            prepare(&mut record, None, 99).unwrap().as_deref(),
            Some(projection.as_str())
        );
    }
    #[test]
    fn malformed_cross_incarnation_or_role_provenance_is_rejected() {
        let mut record = record();
        prepare(&mut record, Some("root"), 0).unwrap();
        record.messages.push(Message::text(Role::User, "root"));
        let valid = record.clone();
        record.incarnation_id = SessionIncarnationId::new("other").unwrap();
        assert!(validate(&record).is_err());
        record = valid.clone();
        record.messages[0].role = Role::Assistant;
        assert!(validate(&record).is_err());
        record = valid;
        record
            .metadata
            .get_mut(NATIVE_PERMISSION_CONTEXT_KEY)
            .unwrap()["recent"] = json!([0]);
        assert!(validate(&record).is_err());
    }
}
