use super::*;
use crate::bounded_profile_file::tests::Fixture;

#[test]
fn mcp_read_rejects_equal_byte_replacement_during_read() {
    let fixture = Fixture::new();
    fixture.write("mcp.json", b"same");
    fixture.write("replacement", b"same");
    let result = read_with_hook(&fixture.descriptor(), ProfileFileKind::Mcp, |_| {
        std::fs::rename(
            fixture.root.join("replacement"),
            fixture.root.join("mcp.json"),
        )
        .unwrap();
    });
    assert_eq!(result.err(), Some(Error::Conflict));
}

#[test]
fn mcp_read_rejects_in_place_change_during_read() {
    let fixture = Fixture::new();
    fixture.write("mcp.json", b"old");
    let result = read_with_hook(&fixture.descriptor(), ProfileFileKind::Mcp, |_| {
        fixture.write("mcp.json", b"new-length");
    });
    assert_eq!(result.err(), Some(Error::Conflict));
}

#[test]
fn mcp_observation_rejects_equal_byte_new_inode_but_settings_remains_byte_cas() {
    for kind in [ProfileFileKind::Mcp, ProfileFileKind::Settings] {
        let fixture = Fixture::new();
        fixture.write(kind.data(), b"same");
        let root = fixture.descriptor();
        let previous = read_current(&root, kind).unwrap();
        fixture.write("replacement", b"same");
        std::fs::rename(
            fixture.root.join("replacement"),
            fixture.root.join(kind.data()),
        )
        .unwrap();
        let current = read_current(&root, kind).unwrap();
        assert_eq!(
            previous.compare(&current, kind),
            if kind == ProfileFileKind::Mcp {
                Err(Error::Conflict)
            } else {
                Ok(())
            }
        );
    }
}
