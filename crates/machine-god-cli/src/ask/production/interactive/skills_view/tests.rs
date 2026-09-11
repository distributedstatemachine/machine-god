use super::*;
use machine_god_native::native_terminal_display_unit_at;

#[test]
fn projection_reports_when_only_a_location_prefix_fits_its_byte_budget() {
    let mut lines = Lines::new(80);
    let label = format!("@ a{}", "\u{301}".repeat(256));
    assert_eq!(lines.push(&label).unwrap(), b"@ ".len());
    assert_eq!(lines.output.finish(), "@ ");
}

fn check_frame(frame: &Frame, columns: u16, rows: u16) -> &str {
    assert!(frame.bytes.len() <= MAX_OUTPUT_BYTES);
    let text = std::str::from_utf8(&frame.bytes).unwrap();
    assert!(!text.contains('\x1b'));
    assert!(!text.ends_with("\r\n"));
    let lines = text.split("\r\n").collect::<Vec<_>>();
    assert!(lines.len() <= usize::from(rows));
    assert!(lines.len() <= MAX_PHYSICAL_ROWS);
    assert_eq!(usize::from(frame.height), lines.len() - 1);
    for line in lines {
        assert!(line.len() <= ROW_BYTES);
        assert!(!line.chars().any(char::is_control));
        let (mut position, mut width) = (0, 0);
        while position < line.len() {
            let unit = native_terminal_display_unit_at(line, position).unwrap();
            position += unit.byte_len;
            width += usize::from(unit.cell_width);
        }
        assert!(width < usize::from(columns), "line wraps: {line:?}");
    }
    text
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "supported.rs"]
mod supported;
