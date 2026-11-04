//! LSP positions count UTF-16 code units; the editor counts chars. Every
//! conversion goes through here, so an emoji can't shift a diagnostic.

pub fn char_to_utf16_col(line: &str, char_col: usize) -> usize {
    line.chars().take(char_col).map(char::len_utf16).sum()
}

/// A column inside a surrogate pair rounds up to the next char.
pub fn utf16_to_char_col(line: &str, utf16_col: usize) -> usize {
    let mut units = 0;
    for (i, c) in line.chars().enumerate() {
        if units >= utf16_col {
            return i;
        }
        units += c.len_utf16();
    }
    line.chars().count()
}
