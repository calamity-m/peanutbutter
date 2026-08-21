//! Conversion between LSP UTF-16 columns and Rust UTF-8 byte offsets.

use tower_lsp::lsp_types::{Position, Range};

/// Convert an LSP UTF-16 column to a byte index in `line`.
///
/// Columns inside a surrogate pair or past the line are rejected. Keeping this
/// conversion at the LSP boundary lets the parser and placeholder scanners
/// continue using byte spans without risking invalid string slices.
pub(super) fn utf16_column_to_byte(line: &str, column: u32) -> Option<usize> {
    let target = column as usize;
    let mut utf16_column = 0usize;

    for (byte_index, ch) in line.char_indices() {
        if utf16_column == target {
            return Some(byte_index);
        }
        let next_column = utf16_column + ch.len_utf16();
        if target < next_column {
            return None;
        }
        utf16_column = next_column;
    }

    (utf16_column == target).then_some(line.len())
}

/// Convert a UTF-16 column while clamping columns past the end of `line`.
///
/// Columns inside a surrogate pair remain invalid. Completion uses this form
/// because requests can race slightly ahead of the server's full-sync content.
pub(super) fn utf16_column_to_byte_clamped(line: &str, column: u32) -> Option<usize> {
    utf16_column_to_byte(line, column)
        .or_else(|| (column > byte_column_to_utf16(line, line.len())).then_some(line.len()))
}

/// Convert a UTF-8 byte index in `line` to an LSP UTF-16 column.
///
/// Byte indexes inside a multi-byte scalar clamp to that scalar's start, while
/// indexes past the line clamp to its end.
pub(super) fn byte_column_to_utf16(line: &str, byte_column: usize) -> u32 {
    let mut boundary = byte_column.min(line.len());
    while !line.is_char_boundary(boundary) {
        boundary -= 1;
    }
    line[..boundary].encode_utf16().count() as u32
}

/// Build an LSP range from a line-local UTF-8 byte span.
pub(super) fn byte_span_to_range(
    line_number: u32,
    line: &str,
    start_byte: usize,
    end_byte: usize,
) -> Range {
    Range {
        start: Position {
            line: line_number,
            character: byte_column_to_utf16(line, start_byte),
        },
        end: Position {
            line: line_number,
            character: byte_column_to_utf16(line, end_byte),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_convert_to_byte_boundaries() {
        let line = "é 名😀x";

        assert_eq!(utf16_column_to_byte(line, 0), Some(0));
        assert_eq!(utf16_column_to_byte(line, 1), Some(2));
        assert_eq!(utf16_column_to_byte(line, 2), Some(3));
        assert_eq!(utf16_column_to_byte(line, 3), Some(6));
        assert_eq!(
            utf16_column_to_byte(line, 4),
            None,
            "inside emoji is invalid"
        );
        assert_eq!(utf16_column_to_byte(line, 5), Some(10));
        assert_eq!(utf16_column_to_byte(line, 6), Some(11));
        assert_eq!(utf16_column_to_byte(line, 100), None);
    }

    #[test]
    fn clamped_conversion_only_clamps_past_end() {
        let line = "😀";

        assert_eq!(utf16_column_to_byte_clamped(line, 1), None);
        assert_eq!(utf16_column_to_byte_clamped(line, 3), Some(line.len()));
    }

    #[test]
    fn byte_columns_convert_to_utf16_boundaries() {
        let line = "é 名😀x";

        assert_eq!(byte_column_to_utf16(line, 0), 0);
        assert_eq!(byte_column_to_utf16(line, 1), 0, "inside é clamps back");
        assert_eq!(byte_column_to_utf16(line, 2), 1);
        assert_eq!(byte_column_to_utf16(line, 3), 2);
        assert_eq!(byte_column_to_utf16(line, 4), 2, "inside 名 clamps back");
        assert_eq!(byte_column_to_utf16(line, 6), 3);
        assert_eq!(byte_column_to_utf16(line, 7), 3, "inside emoji clamps back");
        assert_eq!(byte_column_to_utf16(line, 10), 5);
        assert_eq!(byte_column_to_utf16(line, 11), 6);
        assert_eq!(byte_column_to_utf16(line, 100), 6);
    }

    #[test]
    fn byte_spans_become_utf16_ranges() {
        let line = "é <@name> 😀";
        let range = byte_span_to_range(7, line, 3, 10);

        assert_eq!(range.start, Position::new(7, 2));
        assert_eq!(range.end, Position::new(7, 9));
    }
}
