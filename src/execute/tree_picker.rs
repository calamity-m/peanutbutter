//! Generic tree-shaped picker state used by Browse (file picker) and Tags mode.
//!
//! A [`TreePicker`] is a stack of [`LevelFrame`]s; the topmost frame is the
//! "current level" the user is typing/selecting at. Descending pushes a new
//! frame and records the key of the child we descended into on the *parent*
//! frame. Ascending pops the top frame, leaving the parent's own filter,
//! cursor, and remembered child intact — so the user lands back exactly where
//! they were before descending, with the cursor restored onto the entry they
//! just came from.

/// One level of an interactive tree picker.
#[derive(Debug, Clone, Default)]
pub struct LevelFrame<K> {
    /// Typed filter at this level.
    pub filter: String,
    /// Byte offset of the insertion cursor within `filter`.
    pub cursor: usize,
    /// Highlighted row index in the current visible list, or `None` when the
    /// list is empty.
    pub selection: Option<usize>,
    /// Key of the child this level was last descended into. Set on `descend`
    /// from the parent frame and consumed on `ascend` to restore selection.
    pub descended_from: Option<K>,
}

impl<K> LevelFrame<K> {
    pub fn new() -> Self {
        Self {
            filter: String::new(),
            cursor: 0,
            selection: Some(0),
            descended_from: None,
        }
    }
}

/// Stack-of-frames picker. The current level is `frames.last()`. `path` holds
/// the keys descended into (parallel to `frames[1..]`).
#[derive(Debug, Clone)]
pub struct TreePicker<K> {
    frames: Vec<LevelFrame<K>>,
    path: Vec<K>,
}

impl<K> Default for TreePicker<K> {
    fn default() -> Self {
        Self {
            frames: vec![LevelFrame::new()],
            path: Vec::new(),
        }
    }
}

impl<K: Clone + PartialEq> TreePicker<K> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn depth(&self) -> usize {
        self.path.len()
    }

    pub fn path(&self) -> &[K] {
        &self.path
    }

    pub fn current(&self) -> &LevelFrame<K> {
        self.frames.last().expect("frames is never empty")
    }

    pub fn current_mut(&mut self) -> &mut LevelFrame<K> {
        self.frames.last_mut().expect("frames is never empty")
    }

    pub fn frames_mut(&mut self) -> &mut [LevelFrame<K>] {
        &mut self.frames
    }

    pub fn filter(&self) -> &str {
        &self.current().filter
    }

    pub fn cursor(&self) -> usize {
        self.current().cursor
    }

    pub fn selection(&self) -> Option<usize> {
        self.current().selection
    }

    pub fn set_selection(&mut self, selection: Option<usize>) {
        self.current_mut().selection = selection;
    }

    /// Display-column offset of the cursor within the current filter.
    pub fn cursor_col(&self) -> usize {
        let frame = self.current();
        frame.filter[..frame.cursor].chars().count()
    }

    pub fn type_char(&mut self, c: char) {
        let frame = self.current_mut();
        frame.filter.insert(frame.cursor, c);
        frame.cursor += c.len_utf8();
        frame.selection = Some(0);
    }

    /// Delete one character left of the cursor. Returns `true` if anything
    /// changed (i.e. there was a character to delete).
    pub fn input_backspace(&mut self) -> bool {
        let frame = self.current_mut();
        if frame.cursor == 0 {
            return false;
        }
        let prev = frame.filter[..frame.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        frame.filter.remove(prev);
        frame.cursor = prev;
        frame.selection = Some(0);
        true
    }

    pub fn cursor_left(&mut self) {
        let frame = self.current_mut();
        if frame.cursor == 0 {
            return;
        }
        frame.cursor = frame.filter[..frame.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn cursor_right(&mut self) {
        let frame = self.current_mut();
        if frame.cursor >= frame.filter.len() {
            return;
        }
        let c = frame.filter[frame.cursor..].chars().next().unwrap();
        frame.cursor += c.len_utf8();
    }

    pub fn move_selection(&mut self, delta: i32, visible_len: usize) {
        let frame = self.current_mut();
        if visible_len == 0 {
            frame.selection = None;
            return;
        }
        let current = frame.selection.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, visible_len as i32 - 1);
        frame.selection = Some(next as usize);
    }

    /// Descend into `key`. The current frame retains its filter/cursor and
    /// records `key` as the child it descended into; a fresh frame is pushed
    /// for the new level.
    pub fn descend(&mut self, key: K) {
        self.current_mut().descended_from = Some(key.clone());
        self.path.push(key);
        self.frames.push(LevelFrame::new());
    }

    /// Pop the current level. Returns the popped key, or `None` at the root.
    /// The caller is expected to call [`restore_selection`](Self::restore_selection)
    /// afterwards with the parent's visible list to set the cursor onto the
    /// child we came from.
    pub fn ascend(&mut self) -> Option<K> {
        if self.path.is_empty() {
            return None;
        }
        self.frames.pop();
        self.path.pop()
    }

    /// After an [`ascend`](Self::ascend), set the current frame's selection to
    /// the index of `descended_from` within `visible`. Falls back to `Some(0)`
    /// if the child is not found (e.g. the tree changed or the filter no
    /// longer matches it).
    pub fn restore_selection<T>(&mut self, visible: &[T], key_of: impl Fn(&T) -> Option<K>) {
        let frame = self.current_mut();
        let target = frame.descended_from.take();
        if visible.is_empty() {
            // Keep the conventional "default-to-zero" so renderers that read
            // .unwrap_or(0) behave identically to the pre-refactor code.
            frame.selection = Some(0);
            return;
        }
        if let Some(target) = target
            && let Some(idx) = visible
                .iter()
                .position(|t| key_of(t).as_ref() == Some(&target))
        {
            frame.selection = Some(idx);
            return;
        }
        frame.selection = Some(0);
    }

    /// Reset to the root level with empty filter.
    pub fn reset(&mut self) {
        self.frames.clear();
        self.frames.push(LevelFrame::new());
        self.path.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descend_remembers_child_on_parent_frame() {
        let mut p: TreePicker<String> = TreePicker::new();
        p.type_char('d');
        p.descend("docker".to_string());
        assert_eq!(p.depth(), 1);
        assert_eq!(p.path(), &["docker".to_string()]);
        // Parent frame (now hidden) should still hold "d" and descended_from.
        assert_eq!(p.frames[0].filter, "d");
        assert_eq!(p.frames[0].descended_from.as_deref(), Some("docker"));
    }

    #[test]
    fn ascend_then_restore_places_cursor_on_descended_child() {
        let mut p: TreePicker<String> = TreePicker::new();
        p.type_char('d');
        p.descend("docker".to_string());
        // Now in "docker"; type something then go back up.
        p.type_char('x');
        let popped = p.ascend();
        assert_eq!(popped.as_deref(), Some("docker"));
        // Parent filter preserved.
        assert_eq!(p.filter(), "d");
        let visible = vec!["delta".to_string(), "docker".to_string()];
        p.restore_selection(&visible, |s| Some(s.clone()));
        assert_eq!(p.selection(), Some(1));
    }

    #[test]
    fn restore_falls_back_to_zero_when_child_missing() {
        let mut p: TreePicker<String> = TreePicker::new();
        p.descend("gone".to_string());
        p.ascend();
        let visible = vec!["other".to_string()];
        p.restore_selection(&visible, |s| Some(s.clone()));
        assert_eq!(p.selection(), Some(0));
    }

    #[test]
    fn ascend_at_root_returns_none() {
        let mut p: TreePicker<String> = TreePicker::new();
        assert!(p.ascend().is_none());
        assert_eq!(p.depth(), 0);
    }

    #[test]
    fn input_backspace_only_edits_filter() {
        let mut p: TreePicker<String> = TreePicker::new();
        p.type_char('a');
        p.type_char('b');
        assert!(p.input_backspace());
        assert_eq!(p.filter(), "a");
        assert!(p.input_backspace());
        assert_eq!(p.filter(), "");
        assert!(!p.input_backspace());
    }
}
