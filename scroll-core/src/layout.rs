use crate::SEPARATOR_MARKER_CHAR;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    /// Byte range into the scroll text. Excludes the trailing '\n' of a
    /// newline-terminated line and the leading 0x1E of a separator line.
    pub start: usize,
    pub end: usize,
    pub is_separator: bool,
}

pub struct Layout {
    columns: usize,
    lines: Vec<LineSpan>,
    current_columns: usize, // chars on the last (open) line
}

impl Layout {
    pub fn new(columns: usize) -> Self {
        let mut lines = Vec::new();
        lines.push(LineSpan { start: 0, end: 0, is_separator: false });
        Layout { columns, lines, current_columns: 0 }
    }

    /// Rebuild from the whole scroll (used at boot).
    pub fn from_text(text: &str, columns: usize) -> Self {
        let mut layout = Self::new(columns);
        let mut offset = 0;
        for ch in text.chars() {
            layout.append(offset, ch);
            offset += ch.len_utf8();
        }
        layout
    }

    /// Record one appended char. `byte_offset` is where its bytes start in
    /// the scroll text.
    pub fn append(&mut self, byte_offset: usize, ch: char) {
        if ch == '\n' {
            let after = byte_offset + 1;
            self.lines.push(LineSpan { start: after, end: after, is_separator: false });
            self.current_columns = 0;
            return;
        }
        if ch == SEPARATOR_MARKER_CHAR {
            // Marker is always the first byte of its line (the writer
            // guarantees a preceding '\n'); flag the line and skip the byte.
            let line = self.lines.last_mut().unwrap();
            line.is_separator = true;
            line.start = byte_offset + ch.len_utf8();
            line.end = line.start;
            return;
        }
        if self.current_columns == self.columns {
            self.lines.push(LineSpan { start: byte_offset, end: byte_offset, is_separator: false });
            self.current_columns = 0;
        }
        let line = self.lines.last_mut().unwrap();
        line.end = byte_offset + ch.len_utf8();
        self.current_columns += 1;
    }

    pub fn lines(&self) -> &[LineSpan] {
        &self.lines
    }

    /// Where the cursor sits: (line index, column).
    pub fn cursor(&self) -> (usize, usize) {
        (self.lines.len() - 1, self.current_columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn spans(text: &str, columns: usize) -> Vec<(&str, bool)> {
        Layout::from_text(text, columns)
            .lines()
            .iter()
            .map(|l| (&text[l.start..l.end], l.is_separator))
            .collect()
    }

    #[test]
    fn empty_scroll_is_one_empty_line() {
        assert_eq!(spans("", 10), [("", false)]);
    }

    #[test]
    fn long_lines_wrap_at_columns() {
        assert_eq!(
            spans("abcdefgh", 3),
            [("abc", false), ("def", false), ("gh", false)]
        );
    }

    #[test]
    fn newline_starts_a_new_line() {
        assert_eq!(spans("ab\ncd", 10), [("ab", false), ("cd", false)]);
    }

    #[test]
    fn trailing_newline_leaves_an_open_empty_line() {
        assert_eq!(spans("ab\n", 10), [("ab", false), ("", false)]);
    }

    #[test]
    fn exactly_full_line_then_newline_does_not_double_break() {
        assert_eq!(spans("abc\nd", 3), [("abc", false), ("d", false)]);
    }

    #[test]
    fn separator_marker_dims_the_line_and_is_excluded() {
        let text = "hi\n\u{1E}— 10 June 2026 —\nmore";
        assert_eq!(
            spans(text, 40),
            [("hi", false), ("— 10 June 2026 —", true), ("more", false)]
        );
    }

    #[test]
    fn multibyte_chars_count_as_one_column() {
        // Em dash is 3 bytes but one glyph cell.
        assert_eq!(spans("——a", 2), [("——", false), ("a", false)]);
    }

    #[test]
    fn cursor_tracks_line_and_column() {
        let layout = Layout::from_text("ab\ncd", 10);
        assert_eq!(layout.cursor(), (1, 2));
    }

    #[test]
    fn cursor_after_wrap_is_on_the_new_line() {
        let layout = Layout::from_text("abcd", 3);
        assert_eq!(layout.cursor(), (1, 1));
    }
}
