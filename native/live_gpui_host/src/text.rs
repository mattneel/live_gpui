use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Byte-indexed storage. Every platform-facing coordinate is converted to UTF-16.
#[derive(Default)]
pub struct Text {
    pub value: String,
    pub selection: Range<usize>,
    pub reversed: bool,
    pub marked: Option<Range<usize>>,
    pub revision: u64,
    pub acknowledged: u64,
    deferred: Option<(String, u64)>,
}
pub fn from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units >= offset {
            return byte;
        }
        units += ch.len_utf16();
    }
    text.len()
}
pub fn to_utf16(text: &str, offset: usize) -> usize {
    text.char_indices()
        .take_while(|(byte, _)| *byte < offset)
        .map(|(_, ch)| ch.len_utf16())
        .sum()
}
pub fn utf16_range(text: &str, range: &Range<usize>) -> Range<usize> {
    to_utf16(text, range.start)..to_utf16(text, range.end)
}
pub fn byte_range(text: &str, range: &Range<usize>) -> Range<usize> {
    let start = from_utf16(text, range.start);
    let end = from_utf16(text, range.end).max(start);
    start..end
}
impl Text {
    pub fn new(value: String, revision: u64) -> Self {
        Self {
            value,
            revision,
            acknowledged: revision,
            ..Self::default()
        }
    }
    pub fn caret(&self) -> usize {
        if self.reversed {
            self.selection.start
        } else {
            self.selection.end
        }
    }
    pub fn move_to(&mut self, at: usize, extend: bool) {
        let anchor = if self.reversed {
            self.selection.end
        } else {
            self.selection.start
        };
        if extend {
            self.selection = anchor.min(at)..anchor.max(at);
            self.reversed = at < anchor;
        } else {
            self.selection = at..at;
            self.reversed = false;
        }
    }
    pub fn previous(&self) -> usize {
        self.value
            .grapheme_indices(true)
            .rev()
            .find_map(|(i, _)| (i < self.caret()).then_some(i))
            .unwrap_or(0)
    }
    pub fn next(&self) -> usize {
        self.value
            .grapheme_indices(true)
            .find_map(|(i, _)| (i > self.caret()).then_some(i))
            .unwrap_or(self.value.len())
    }
    pub fn replace(
        &mut self,
        range: Option<Range<usize>>,
        value: &str,
        marked: bool,
        selected: Option<Range<usize>>,
    ) {
        let range = range
            .map(|r| byte_range(&self.value, &r))
            .or_else(|| self.marked.clone())
            .unwrap_or_else(|| self.selection.clone());
        self.value.replace_range(range.clone(), value);
        self.marked =
            (marked && !value.is_empty()).then_some(range.start..range.start + value.len());
        // IME selection is relative to the inserted text, not the old replacement range.
        self.selection = selected
            .map(|r| {
                let r = byte_range(value, &r);
                range.start + r.start..range.start + r.end
            })
            .unwrap_or(range.start + value.len()..range.start + value.len());
        self.reversed = false;
        self.revision = self
            .revision
            .checked_add(1)
            .expect("native edit revision exhausted");
    }
    pub fn reconcile(&mut self, value: &str, revision: u64) {
        if revision < self.acknowledged || revision < self.revision {
            return;
        }
        self.acknowledged = revision;
        self.revision = revision;
        if self.value == value {
            self.deferred = None;
            return;
        }
        if self.marked.is_some() {
            self.deferred = Some((value.to_owned(), revision));
            return;
        }
        self.value.clear();
        self.value.push_str(value);
        let at = self.value.len();
        self.selection = at..at;
        self.reversed = false;
    }
    pub fn unmark(&mut self) {
        self.marked = None;
        if let Some((value, revision)) = self.deferred.take() {
            self.reconcile(&value, revision);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf16_surrogates_and_combining_graphemes() {
        assert_eq!(from_utf16("a😀z", 2), 5);
        assert_eq!(to_utf16("a😀z", 5), 3);
        let mut t = Text::new("a\u{301}😀".into(), 0);
        t.move_to(t.value.len(), false);
        assert_eq!(t.previous(), 3);
        t.move_to(3, false);
        assert_eq!(t.previous(), 0);
    }
    #[test]
    fn ime_selection_is_relative_to_new_text() {
        let mut t = Text::new("prefixOLDsuffix".into(), 0);
        t.selection = 6..9;
        t.replace(None, "😀ab", true, Some(2..3));
        assert_eq!(t.selection, 10..11);
        assert_eq!(t.marked, Some(6..12));
    }
    #[test]
    fn old_echo_and_equal_echo_preserve_caret() {
        let mut t = Text::new("a".into(), 0);
        t.move_to(1, false);
        t.replace(None, "b", false, None);
        t.move_to(0, false);
        t.reconcile("a", 0);
        assert_eq!(t.value, "ab");
        t.reconcile("ab", 1);
        assert_eq!(t.selection, 0..0);
    }
    #[test]
    fn acknowledgement_does_not_destroy_composition() {
        let mut t = Text::default();
        t.replace(None, "あ", true, Some(0..1));
        t.reconcile("server", 1);
        assert_eq!(t.value, "あ");
        assert_eq!(t.marked, Some(0..3));
        t.reconcile("old", 0);
        assert_eq!(t.value, "あ");
        t.unmark();
        assert_eq!(t.value, "server");
        assert_eq!(t.marked, None);
    }
    #[test]
    fn newer_native_edit_supersedes_deferred_application_value() {
        let mut t = Text::default();
        t.replace(None, "あ", true, None);
        t.reconcile("server", 1);
        t.replace(None, "い", true, None);
        t.unmark();
        assert_eq!(t.value, "い");
        assert_eq!(t.revision, 2);
    }
}
