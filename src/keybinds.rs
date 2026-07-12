//! Customizable keybinds for the `pb execute`, `pb settings`, and `pb new`
//! TUIs.
//!
//! Users remap actions through `[keybinds.<command>.<context>]` TOML tables
//! (`execute`, `settings`, or `new`); each action maps to an array of key
//! chord strings. Omitted actions keep their defaults, an empty array unbinds
//! the action, and invalid chords are collected as non-fatal warnings so the
//! rest of the config still applies.
//!
//! `Ctrl+C` is an app-level emergency cancel handled before the keymap and is
//! rejected as a configurable chord — raw-mode terminals need one unconditional
//! escape hatch that no config can remove.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fmt;

/// A single key chord: one base key plus ctrl/alt/shift modifiers, in the
/// canonical shape crossterm delivers.
///
/// Canonicalization rules (applied by both [`KeyChord::parse`] and
/// [`KeyChord::from_event`] so config strings and live events compare equal):
/// - `shift+<char>` becomes the uppercase character with SHIFT dropped,
///   matching how crossterm reports shifted printable keys.
/// - `shift+tab` and `backtab` both become [`KeyCode::BackTab`] with SHIFT
///   dropped.
/// - Modifiers other than ctrl/alt/shift (e.g. SUPER) are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { code, modifiers }
    }

    /// The reserved emergency-cancel chord that user config may not bind.
    pub fn reserved_cancel() -> Self {
        Self::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
    }

    /// Canonicalize a live crossterm key event for keymap lookup.
    pub fn from_event(event: &KeyEvent) -> Self {
        let modifiers =
            event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
        Self::canonicalize(event.code, modifiers)
    }

    /// Parse a chord string like `ctrl+shift+x`, `alt+enter`, or `f2`.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("empty key string".to_string());
        }
        let mut modifiers = KeyModifiers::NONE;
        let parts: Vec<&str> = raw.split('+').collect();
        let (mods, base) = parts.split_at(parts.len() - 1);
        for part in mods {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "alt" => modifiers |= KeyModifiers::ALT,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                other => return Err(format!("unknown modifier `{other}` in `{raw}`")),
            }
        }
        let code = parse_base_key(base[0], raw)?;
        Ok(Self::canonicalize(code, modifiers))
    }

    fn canonicalize(code: KeyCode, mut modifiers: KeyModifiers) -> Self {
        let code = match code {
            // Crossterm reports shift+a as Char('A')+SHIFT; fold the modifier
            // into the character so both spellings compare equal.
            KeyCode::Char(c) if modifiers.contains(KeyModifiers::SHIFT) => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c.to_ascii_uppercase())
            }
            KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            KeyCode::BackTab => {
                modifiers.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Self::new(code, modifiers)
    }
}

fn parse_base_key(token: &str, raw: &str) -> Result<KeyCode, String> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(KeyCode::Char(c));
    }
    let code = match token.to_ascii_lowercase().as_str() {
        // `space` avoids an easy-to-misread literal `" "` in TOML, and `plus`
        // exists because a bare `"+"` cannot parse: chords split on `'+'`, so
        // it would produce an empty base token.
        "space" => KeyCode::Char(' '),
        "plus" => KeyCode::Char('+'),
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "insert" => KeyCode::Insert,
        "delete" => KeyCode::Delete,
        fkey if fkey.starts_with('f') => {
            let n: u8 = fkey[1..]
                .parse()
                .map_err(|_| format!("unknown key `{token}` in `{raw}`"))?;
            if !(1..=12).contains(&n) {
                return Err(format!("unknown key `{token}` in `{raw}`"));
            }
            KeyCode::F(n)
        }
        _ => return Err(format!("unknown key `{token}` in `{raw}`")),
    };
    Ok(code)
}

impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            f.write_str("ctrl+")?;
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            f.write_str("alt+")?;
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            f.write_str("shift+")?;
        }
        match self.code {
            KeyCode::Char(' ') => f.write_str("space"),
            KeyCode::Char('+') => f.write_str("plus"),
            KeyCode::Char(c) => write!(f, "{c}"),
            KeyCode::Enter => f.write_str("enter"),
            KeyCode::Esc => f.write_str("esc"),
            KeyCode::Backspace => f.write_str("backspace"),
            KeyCode::Tab => f.write_str("tab"),
            KeyCode::BackTab => f.write_str("shift+tab"),
            KeyCode::Up => f.write_str("up"),
            KeyCode::Down => f.write_str("down"),
            KeyCode::Left => f.write_str("left"),
            KeyCode::Right => f.write_str("right"),
            KeyCode::PageUp => f.write_str("pageup"),
            KeyCode::PageDown => f.write_str("pagedown"),
            KeyCode::Home => f.write_str("home"),
            KeyCode::End => f.write_str("end"),
            KeyCode::Insert => f.write_str("insert"),
            KeyCode::Delete => f.write_str("delete"),
            KeyCode::F(n) => write!(f, "f{n}"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// One configurable action within a keybind context.
///
/// `ALL` is the documented conflict-precedence order: when one chord is bound
/// to two actions in the same context, the action earlier in `ALL` keeps it.
pub trait KeymapAction: Copy + PartialEq + Sized + 'static {
    /// Context name as written in the config (`select`, `fuzzy`, ...).
    const CONTEXT: &'static str;
    /// All actions in documented precedence order.
    const ALL: &'static [Self];
    /// Action name as written in the config.
    fn name(self) -> &'static str;
    /// Default chord strings (must parse; covered by tests).
    fn default_keys(self) -> &'static [&'static str];
}

/// Declares one keybind context: an action enum plus its [`KeymapAction`]
/// impl, from a single table so the variant list, config names, and default
/// chords cannot drift apart.
///
/// An invocation like
///
/// ```text
/// keymap_actions!(
///     /// Doc comment attached to the generated enum.
///     SelectAction, "select", {
///         CancelOrBack => "cancel_or_back", ["esc"];
///         CycleMode => "cycle_mode", ["ctrl+t", "f2"];
///     }
/// );
/// ```
///
/// expands to exactly this hand-written equivalent:
///
/// ```text
/// /// Doc comment attached to the generated enum.
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// pub enum SelectAction { CancelOrBack, CycleMode }
///
/// impl KeymapAction for SelectAction {
///     const CONTEXT: &'static str = "select";
///     const ALL: &'static [Self] = &[Self::CancelOrBack, Self::CycleMode];
///     fn name(self) -> &'static str { /* "cancel_or_back" | "cycle_mode" */ }
///     fn default_keys(self) -> &'static [&'static str] { /* ["esc"] | ... */ }
/// }
/// ```
///
/// Row order is load-bearing: `ALL` preserves it, and conflict resolution
/// gives earlier actions ownership of a duplicated chord. The enum shape is
/// checked at compile time, but the *strings* are only data — bad default
/// chords surface as a panic in `ContextBindings::default()`, which the
/// `all_default_keybinds_parse` test exercises for every context.
macro_rules! keymap_actions {
    ($(#[$meta:meta])* $enum_name:ident, $context:literal,
     { $($variant:ident => $name:literal, [$($key:literal),*]);+ $(;)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $enum_name {
            $($variant,)+
        }

        impl KeymapAction for $enum_name {
            const CONTEXT: &'static str = $context;
            const ALL: &'static [Self] = &[$(Self::$variant,)+];

            fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            fn default_keys(self) -> &'static [&'static str] {
                match self {
                    $(Self::$variant => &[$($key),*],)+
                }
            }
        }
    };
}

keymap_actions!(
    /// Actions handled on the select screen before mode-specific dispatch.
    SelectAction, "select", {
        CancelOrBack => "cancel_or_back", ["esc"];
        CycleMode => "cycle_mode", ["ctrl+t", "f2"];
        Edit => "edit", ["ctrl+e"];
        PreviewDown => "preview_down", ["ctrl+j", "ctrl+down"];
        PreviewUp => "preview_up", ["ctrl+k", "ctrl+up"];
    }
);

keymap_actions!(
    /// Fuzzy search list movement and selection.
    FuzzyAction, "fuzzy", {
        Accept => "accept", ["enter"];
        Backspace => "backspace", ["backspace"];
        CursorLeft => "cursor_left", ["left"];
        CursorRight => "cursor_right", ["right"];
        MoveUp => "move_up", ["up"];
        MoveDown => "move_down", ["down"];
        PageUp => "page_up", ["pageup"];
        PageDown => "page_down", ["pagedown"];
    }
);

keymap_actions!(
    /// File tree movement, completion, opening, and selection.
    BrowseAction, "browse", {
        AcceptOrOpen => "accept_or_open", ["enter"];
        Backspace => "backspace", ["backspace"];
        Complete => "complete", ["tab"];
        CursorLeft => "cursor_left", ["left"];
        CursorRight => "cursor_right", ["right"];
        MoveUp => "move_up", ["up"];
        MoveDown => "move_down", ["down"];
        PageUp => "page_up", ["pageup"];
        PageDown => "page_down", ["pagedown"];
    }
);

keymap_actions!(
    /// Tag list and drilled tag movement/selection.
    TagsAction, "tags", {
        AcceptOrDrill => "accept_or_drill", ["enter"];
        Backspace => "backspace", ["backspace"];
        CursorLeft => "cursor_left", ["left"];
        CursorRight => "cursor_right", ["right"];
        MoveUp => "move_up", ["up"];
        MoveDown => "move_down", ["down"];
        PageUp => "page_up", ["pageup"];
        PageDown => "page_down", ["pagedown"];
        ReturnToTags => "return_to_tags", ["esc"];
    }
);

keymap_actions!(
    /// Variable prompt navigation, suggestion movement, accept, and newline.
    PromptAction, "prompt", {
        ReturnToPicker => "return_to_picker", ["esc"];
        BackspaceOrPrevious => "backspace_or_previous", ["backspace"];
        LiteralNewline => "literal_newline", ["alt+enter", "ctrl+j"];
        SuggestionUp => "suggestion_up", ["up"];
        SuggestionDown => "suggestion_down", ["down"];
        CompleteOrNext => "complete_or_next", ["tab"];
        PreviousVariable => "previous_variable", ["shift+tab", "backtab"];
        Accept => "accept", ["enter"];
    }
);

keymap_actions!(
    /// Actions handled before `pb settings` screen dispatch.
    SettingsGlobalAction, "global", {
        Quit => "quit", ["q", "shift+q"];
    }
);

keymap_actions!(
    /// Shared list actions for the settings section, theme, and paths screens.
    /// `reset` only takes effect on the theme screen (reset to the default
    /// theme and save); the other screens ignore it.
    SettingsListAction, "list", {
        Back => "back", ["esc", "backspace"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        Select => "select", ["enter"];
        Reset => "reset", ["r", "shift+r"];
    }
);

keymap_actions!(
    /// Search tuner chooser screen: list actions plus group reset.
    SettingsSearchAction, "search", {
        Back => "back", ["esc", "backspace"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        Select => "select", ["enter"];
        Reset => "reset", ["r", "shift+r"];
    }
);

keymap_actions!(
    /// Slider editor for one search group.
    SettingsTunerAction, "tuner", {
        Back => "back", ["esc", "backspace"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        Decrease => "decrease", ["left", "-"];
        Increase => "increase", ["right", "plus", "="];
        Reset => "reset", ["r", "shift+r"];
        Save => "save", ["enter"];
    }
);

keymap_actions!(
    /// Keybind editor action-list operations.
    SettingsKeybindsAction, "keybinds", {
        Back => "back", ["esc", "backspace"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        ChordLeft => "chord_left", ["left"];
        ChordRight => "chord_right", ["right"];
        Capture => "capture", ["enter", "a"];
        DeleteChord => "delete_chord", ["d", "delete"];
        Reset => "reset", ["r", "shift+r"];
        Unbind => "unbind", ["u"];
        Save => "save", ["s"];
    }
);

keymap_actions!(
    /// Shared by the `pb new` history picker and target picker.
    /// `cancel_or_back` cancels the capture in the history picker and steps
    /// back to the confirm screen in the target picker.
    NewPickerAction, "picker", {
        Accept => "accept", ["enter"];
        CancelOrBack => "cancel_or_back", ["esc"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        Backspace => "backspace", ["backspace"];
    }
);

keymap_actions!(
    /// Snippet-name input on the `pb new` confirm screen. This widget always
    /// accepts text, so unmodified printable chords never resolve here.
    NewConfirmNameAction, "confirm_name", {
        Accept => "accept", ["enter"];
        Cancel => "cancel", ["esc"];
        CompleteOrFocusTokens => "complete_or_focus_tokens", ["tab", "down"];
        Backspace => "backspace", ["backspace"];
    }
);

keymap_actions!(
    /// Token list on the `pb new` confirm screen.
    NewConfirmTokensAction, "confirm_tokens", {
        Cancel => "cancel", ["esc"];
        Back => "back", ["b"];
        MoveUp => "move_up", ["up", "k"];
        MoveDown => "move_down", ["down", "j"];
        ToggleVariable => "toggle_variable", ["space"];
        Rename => "rename", ["e"];
        EditName => "edit_name", ["n"];
        Accept => "accept", ["enter"];
    }
);

keymap_actions!(
    /// Token rename input on the `pb new` confirm screen. Always accepts
    /// text, so unmodified printable chords never resolve here.
    NewConfirmRenameAction, "confirm_rename", {
        Cancel => "cancel", ["esc"];
        Accept => "accept", ["enter"];
        Backspace => "backspace", ["backspace"];
    }
);

/// How the widget resolving a key treats plain text input. This is the single
/// implementation of the plain-letter precedence rule: widgets that can
/// receive typed text must not let letter bindings eat keystrokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEntry {
    /// The widget never accepts text: keymap resolution runs unconditionally.
    None,
    /// The widget accepts text conditionally (a picker filter): unmodified
    /// printable chords resolve as actions only while the input is empty
    /// (`true` = currently empty); otherwise they are text.
    WhenEmpty(bool),
    /// The widget always accepts text (name/rename fields): unmodified
    /// printable chords never resolve as actions — such bindings are inert
    /// by design.
    Always,
}

/// Resolved bindings for one context: a chord list per action, indexed in
/// [`KeymapAction::ALL`] order.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextBindings<A: KeymapAction> {
    bindings: Vec<Vec<KeyChord>>,
    _actions: std::marker::PhantomData<A>,
}

impl<A: KeymapAction> Default for ContextBindings<A> {
    fn default() -> Self {
        let bindings = A::ALL
            .iter()
            .map(|action| {
                let mut chords: Vec<KeyChord> = action
                    .default_keys()
                    .iter()
                    .map(|raw| {
                        KeyChord::parse(raw).expect("default keybinds are valid chord strings")
                    })
                    .collect();
                chords.dedup();
                chords
            })
            .collect();
        Self {
            bindings,
            _actions: std::marker::PhantomData,
        }
    }
}

impl<A: KeymapAction> ContextBindings<A> {
    /// Resolved chords for an action, in config/display order.
    pub fn chords(&self, action: A) -> &[KeyChord] {
        &self.bindings[Self::index_of(action)]
    }

    /// Resolve a canonical chord to the first matching action in precedence
    /// order.
    pub fn action(&self, chord: &KeyChord) -> Option<A> {
        A::ALL
            .iter()
            .zip(&self.bindings)
            .find(|(_, chords)| chords.contains(chord))
            .map(|(action, _)| *action)
    }

    /// Convenience: resolve a live key event.
    pub fn action_for(&self, event: &KeyEvent) -> Option<A> {
        self.action(&KeyChord::from_event(event))
    }

    /// Resolve a live key event under the [`TextEntry`] precedence rule.
    ///
    /// Note the check runs on the *canonical* chord, so a shifted letter
    /// (`shift+a` → `A` with SHIFT folded away) still counts as typed text.
    pub fn resolve(&self, event: &KeyEvent, text_entry: TextEntry) -> Option<A> {
        let chord = KeyChord::from_event(event);
        let plain_printable = matches!(chord.code, KeyCode::Char(_)) && chord.modifiers.is_empty();
        let text_wins = match text_entry {
            TextEntry::None => false,
            TextEntry::WhenEmpty(input_is_empty) => plain_printable && !input_is_empty,
            TextEntry::Always => plain_printable,
        };
        if text_wins { None } else { self.action(&chord) }
    }

    /// The preferred display chord for an action, or `None` when the action
    /// has been intentionally unbound (dynamic help must then omit it).
    pub fn hint(&self, action: A) -> Option<String> {
        let idx = A::ALL.iter().position(|a| *a == action)?;
        self.bindings[idx].first().map(|chord| chord.to_string())
    }

    fn index_of(action: A) -> usize {
        A::ALL
            .iter()
            .position(|a| *a == action)
            .expect("action is a member of ALL")
    }

    /// Apply one `[keybinds.<section>.<context>]` table. Invalid entries
    /// become warnings; valid entries replace that action's defaults; an
    /// empty array unbinds the action.
    fn apply(&mut self, section: &str, table: &toml::value::Table, warnings: &mut Vec<String>) {
        for (name, value) in table {
            let Some(action) = A::ALL.iter().find(|action| action.name() == name) else {
                warnings.push(format!(
                    "keybinds: unknown action `{name}` in [keybinds.{section}.{}]",
                    A::CONTEXT
                ));
                continue;
            };
            let Some(raw_chords) = value.as_array() else {
                warnings.push(format!(
                    "keybinds: expected an array of key strings for {section}.{}.{name}",
                    A::CONTEXT
                ));
                continue;
            };
            if raw_chords.is_empty() {
                self.bindings[Self::index_of(*action)] = Vec::new();
                continue;
            }
            let mut chords = Vec::new();
            for raw in raw_chords {
                let Some(raw) = raw.as_str() else {
                    warnings.push(format!(
                        "keybinds: expected a key string for {section}.{}.{name}, got {raw}",
                        A::CONTEXT
                    ));
                    continue;
                };
                match KeyChord::parse(raw) {
                    Ok(chord) if chord == KeyChord::reserved_cancel() => {
                        warnings.push(format!(
                            "keybinds: `{raw}` is reserved for emergency cancel and cannot be bound ({section}.{}.{name})",
                            A::CONTEXT
                        ));
                    }
                    Ok(chord) => {
                        if !chords.contains(&chord) {
                            chords.push(chord);
                        }
                    }
                    Err(err) => {
                        warnings.push(format!(
                            "keybinds: invalid key for {section}.{}.{name}: {err}",
                            A::CONTEXT
                        ));
                    }
                }
            }
            if chords.is_empty() {
                warnings.push(format!(
                    "keybinds: no valid keys configured for {section}.{}.{name}; keeping defaults",
                    A::CONTEXT
                ));
            } else {
                self.bindings[Self::index_of(*action)] = chords;
            }
        }
    }

    /// Drop chords already owned by an earlier action in precedence order,
    /// warning about each ignored duplicate.
    fn resolve_conflicts(&mut self, section: &str, warnings: &mut Vec<String>) {
        let mut seen: Vec<(KeyChord, &'static str)> = Vec::new();
        for (action, chords) in A::ALL.iter().zip(&mut self.bindings) {
            chords.retain(|chord| {
                if let Some((_, owner)) = seen.iter().find(|(c, _)| c == chord) {
                    warnings.push(format!(
                        "keybinds: duplicate key `{chord}` in {section}.{} ignored for {} (already bound to {owner})",
                        A::CONTEXT,
                        action.name(),
                    ));
                    false
                } else {
                    seen.push((*chord, action.name()));
                    true
                }
            });
        }
    }
}

/// Validate one `[keybinds.<section>.<context>]` value as a table, warning
/// when it is not.
fn context_table<'v>(
    section: &str,
    context: &str,
    value: &'v toml::Value,
    warnings: &mut Vec<String>,
) -> Option<&'v toml::value::Table> {
    let table = value.as_table();
    if table.is_none() {
        warnings.push(format!(
            "keybinds: expected [keybinds.{section}.{context}] to be a table"
        ));
    }
    table
}

/// The full resolved keymap for the `pb execute` TUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecuteKeymap {
    pub select: ContextBindings<SelectAction>,
    pub fuzzy: ContextBindings<FuzzyAction>,
    pub browse: ContextBindings<BrowseAction>,
    pub tags: ContextBindings<TagsAction>,
    pub prompt: ContextBindings<PromptAction>,
}

impl ExecuteKeymap {
    /// `true` when the event is the hard, non-configurable emergency cancel.
    pub fn is_emergency_cancel(event: &KeyEvent) -> bool {
        is_emergency_cancel(event)
    }

    fn apply_section(&mut self, value: &toml::Value, warnings: &mut Vec<String>) {
        let Some(contexts) = value.as_table() else {
            warnings.push("keybinds: expected [keybinds.execute] to be a table".to_string());
            return;
        };
        for (context, actions) in contexts {
            let Some(actions) = context_table("execute", context, actions, warnings) else {
                continue;
            };
            match context.as_str() {
                "select" => self.select.apply("execute", actions, warnings),
                "fuzzy" => self.fuzzy.apply("execute", actions, warnings),
                "browse" => self.browse.apply("execute", actions, warnings),
                "tags" => self.tags.apply("execute", actions, warnings),
                "prompt" => self.prompt.apply("execute", actions, warnings),
                other => warnings.push(format!(
                    "keybinds: unknown context `execute.{other}`; expected one of: select, fuzzy, browse, tags, prompt"
                )),
            }
        }
    }

    fn resolve_conflicts(&mut self, warnings: &mut Vec<String>) {
        self.select.resolve_conflicts("execute", warnings);
        self.fuzzy.resolve_conflicts("execute", warnings);
        self.browse.resolve_conflicts("execute", warnings);
        self.tags.resolve_conflicts("execute", warnings);
        self.prompt.resolve_conflicts("execute", warnings);
    }
}

/// `true` when the event is the hard, non-configurable emergency cancel
/// (`ctrl+c`) shared by every pb TUI.
pub fn is_emergency_cancel(event: &KeyEvent) -> bool {
    KeyChord::from_event(event) == KeyChord::reserved_cancel()
}

/// Format a `"{key} {label}"` footer hint for a bound action, or `None` when
/// the action is unbound so dynamic help omits it.
pub fn help_hint(key: Option<String>, label: &str) -> Option<String> {
    key.map(|k| format!("{k} {label}"))
}

/// Paired movement hint like `up/down move`; degrades to whichever side is
/// still bound, or disappears when both are unbound.
pub fn help_move_hint(up: Option<String>, down: Option<String>, label: &str) -> Option<String> {
    match (up, down) {
        (Some(u), Some(d)) => Some(format!("{u}/{d} {label}")),
        (Some(k), None) | (None, Some(k)) => Some(format!("{k} {label}")),
        (None, None) => None,
    }
}

/// The full resolved keymap for the `pb settings` TUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsKeymap {
    pub global: ContextBindings<SettingsGlobalAction>,
    pub list: ContextBindings<SettingsListAction>,
    pub search: ContextBindings<SettingsSearchAction>,
    pub tuner: ContextBindings<SettingsTunerAction>,
    pub keybinds: ContextBindings<SettingsKeybindsAction>,
}

impl SettingsKeymap {
    fn apply_section(&mut self, value: &toml::Value, warnings: &mut Vec<String>) {
        let Some(contexts) = value.as_table() else {
            warnings.push("keybinds: expected [keybinds.settings] to be a table".to_string());
            return;
        };
        for (context, actions) in contexts {
            let Some(actions) = context_table("settings", context, actions, warnings) else {
                continue;
            };
            match context.as_str() {
                "global" => self.global.apply("settings", actions, warnings),
                "list" => self.list.apply("settings", actions, warnings),
                "search" => self.search.apply("settings", actions, warnings),
                "tuner" => self.tuner.apply("settings", actions, warnings),
                "keybinds" => self.keybinds.apply("settings", actions, warnings),
                other => warnings.push(format!(
                    "keybinds: unknown context `settings.{other}`; expected one of: global, list, search, tuner, keybinds"
                )),
            }
        }
    }

    fn resolve_conflicts(&mut self, warnings: &mut Vec<String>) {
        self.global.resolve_conflicts("settings", warnings);
        self.list.resolve_conflicts("settings", warnings);
        self.search.resolve_conflicts("settings", warnings);
        self.tuner.resolve_conflicts("settings", warnings);
        self.keybinds.resolve_conflicts("settings", warnings);
    }
}

/// The full resolved keymap for the `pb new` capture TUI.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewKeymap {
    pub picker: ContextBindings<NewPickerAction>,
    pub confirm_name: ContextBindings<NewConfirmNameAction>,
    pub confirm_tokens: ContextBindings<NewConfirmTokensAction>,
    pub confirm_rename: ContextBindings<NewConfirmRenameAction>,
}

impl NewKeymap {
    fn apply_section(&mut self, value: &toml::Value, warnings: &mut Vec<String>) {
        let Some(contexts) = value.as_table() else {
            warnings.push("keybinds: expected [keybinds.new] to be a table".to_string());
            return;
        };
        for (context, actions) in contexts {
            let Some(actions) = context_table("new", context, actions, warnings) else {
                continue;
            };
            match context.as_str() {
                "picker" => self.picker.apply("new", actions, warnings),
                "confirm_name" => self.confirm_name.apply("new", actions, warnings),
                "confirm_tokens" => self.confirm_tokens.apply("new", actions, warnings),
                "confirm_rename" => self.confirm_rename.apply("new", actions, warnings),
                other => warnings.push(format!(
                    "keybinds: unknown context `new.{other}`; expected one of: picker, confirm_name, confirm_tokens, confirm_rename"
                )),
            }
        }
    }

    fn resolve_conflicts(&mut self, warnings: &mut Vec<String>) {
        self.picker.resolve_conflicts("new", warnings);
        self.confirm_name.resolve_conflicts("new", warnings);
        self.confirm_tokens.resolve_conflicts("new", warnings);
        self.confirm_rename.resolve_conflicts("new", warnings);
    }
}

/// Every resolved keymap plus the diagnostics produced while resolving them.
///
/// This is the output of interpreting the raw `[keybinds]` config value —
/// one keymap per interactive command alongside the non-fatal warnings
/// resolution produced. The warning list is shared: every TUI shows the full
/// list, so a settings typo is visible from `pb execute` too. Warnings live
/// here rather than as standalone config because they describe this
/// resolution, and each TUI decides how to surface them (`pb execute` shows
/// them as status so they never touch the stdout command payload).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymaps {
    /// Resolved keybinds for the `pb execute` TUI.
    pub execute: ExecuteKeymap,
    /// Resolved keybinds for the `pb settings` TUI.
    pub settings: SettingsKeymap,
    /// Resolved keybinds for the `pb new` capture TUI.
    pub new: NewKeymap,
    /// Non-fatal validation warnings: invalid or reserved chords, unknown
    /// contexts/actions, wrong value types, and ignored duplicate bindings.
    pub warnings: Vec<String>,
}

impl Keymaps {
    /// Resolve the raw `[keybinds]` config value. `None` (no `[keybinds]`
    /// section) yields pure defaults with no warnings.
    pub fn resolve(raw: Option<&toml::Value>) -> Self {
        let mut execute = ExecuteKeymap::default();
        let mut settings = SettingsKeymap::default();
        let mut new = NewKeymap::default();
        let mut warnings = Vec::new();
        if let Some(raw) = raw {
            match raw.as_table() {
                Some(table) => {
                    for (section, value) in table {
                        match section.as_str() {
                            "execute" => execute.apply_section(value, &mut warnings),
                            "settings" => settings.apply_section(value, &mut warnings),
                            "new" => new.apply_section(value, &mut warnings),
                            other => warnings
                                .push(format!("keybinds: unknown section `keybinds.{other}`")),
                        }
                    }
                }
                None => warnings.push("keybinds: expected [keybinds] to be a table".to_string()),
            }
        }
        execute.resolve_conflicts(&mut warnings);
        settings.resolve_conflicts(&mut warnings);
        new.resolve_conflicts(&mut warnings);
        Self {
            execute,
            settings,
            new,
            warnings,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve_toml(raw: &str) -> (ExecuteKeymap, Vec<String>) {
        let value: toml::Value = toml::from_str(raw).unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        (keymaps.execute, keymaps.warnings)
    }

    #[test]
    fn parse_canonicalizes_modifiers_and_case() {
        assert_eq!(
            KeyChord::parse("CTRL+T").unwrap(),
            KeyChord::new(KeyCode::Char('T'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            KeyChord::parse("shift+a").unwrap(),
            KeyChord::new(KeyCode::Char('A'), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyChord::parse("shift+tab").unwrap(),
            KeyChord::parse("backtab").unwrap()
        );
        assert_eq!(
            KeyChord::parse("ctrl+shift+x").unwrap(),
            KeyChord::new(KeyCode::Char('X'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            KeyChord::parse("alt+enter").unwrap(),
            KeyChord::new(KeyCode::Enter, KeyModifiers::ALT)
        );
        assert_eq!(
            KeyChord::parse("f2").unwrap(),
            KeyChord::new(KeyCode::F(2), KeyModifiers::NONE)
        );
    }

    #[test]
    fn parse_rejects_unknown_keys_and_modifiers() {
        assert!(KeyChord::parse("").is_err());
        assert!(KeyChord::parse("hyper+x").is_err());
        assert!(KeyChord::parse("ctrl+widget").is_err());
        assert!(KeyChord::parse("f13").is_err());
        assert!(KeyChord::parse("f0").is_err());
    }

    #[test]
    fn display_is_canonical_and_round_trips() {
        for raw in [
            "ctrl+t",
            "alt+enter",
            "shift+up",
            "esc",
            "pagedown",
            "f12",
            "ctrl+j",
        ] {
            let chord = KeyChord::parse(raw).unwrap();
            assert_eq!(KeyChord::parse(&chord.to_string()).unwrap(), chord);
        }
        assert_eq!(KeyChord::parse("ctrl+t").unwrap().to_string(), "ctrl+t");
        assert_eq!(KeyChord::parse("backtab").unwrap().to_string(), "shift+tab");
    }

    #[test]
    fn from_event_matches_parsed_defaults() {
        let event = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
        assert_eq!(
            KeyChord::from_event(&event),
            KeyChord::parse("ctrl+t").unwrap()
        );
        // Shifted printable char as crossterm reports it.
        let event = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(
            KeyChord::from_event(&event),
            KeyChord::parse("shift+a").unwrap()
        );
        // BackTab arrives with SHIFT on some terminals.
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(
            KeyChord::from_event(&event),
            KeyChord::parse("shift+tab").unwrap()
        );
    }

    #[test]
    fn all_default_keybinds_parse() {
        // ContextBindings::default() panics on invalid default strings; touch
        // every context to prove the tables are well-formed.
        let keymap = ExecuteKeymap::default();
        assert_eq!(
            keymap.select.hint(SelectAction::CycleMode).unwrap(),
            "ctrl+t"
        );
        assert_eq!(keymap.fuzzy.hint(FuzzyAction::Accept).unwrap(), "enter");
        assert_eq!(keymap.browse.hint(BrowseAction::Complete).unwrap(), "tab");
        assert_eq!(keymap.tags.hint(TagsAction::ReturnToTags).unwrap(), "esc");
        assert_eq!(
            keymap.prompt.hint(PromptAction::PreviousVariable).unwrap(),
            "shift+tab"
        );
    }

    #[test]
    fn omitted_actions_keep_defaults_and_custom_replaces() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.select]
cycle_mode = ["ctrl+n"]
"#,
        );
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(
            keymap
                .select
                .action(&KeyChord::parse("ctrl+n").unwrap())
                .unwrap(),
            SelectAction::CycleMode
        );
        // Replaced defaults for cycle_mode only.
        assert_eq!(
            keymap.select.action(&KeyChord::parse("ctrl+t").unwrap()),
            None
        );
        // Other actions untouched.
        assert_eq!(
            keymap.select.action(&KeyChord::parse("ctrl+e").unwrap()),
            Some(SelectAction::Edit)
        );
    }

    #[test]
    fn empty_array_unbinds_action() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.select]
edit = []
"#,
        );
        assert!(warnings.is_empty());
        assert_eq!(
            keymap.select.action(&KeyChord::parse("ctrl+e").unwrap()),
            None
        );
        assert_eq!(keymap.select.hint(SelectAction::Edit), None);
    }

    #[test]
    fn all_invalid_chords_keep_defaults_with_warning() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.fuzzy]
accept = ["hyper+x", "notakey"]
"#,
        );
        assert_eq!(
            keymap.fuzzy.action(&KeyChord::parse("enter").unwrap()),
            Some(FuzzyAction::Accept)
        );
        assert!(warnings.iter().any(|w| w.contains("invalid key")));
        assert!(warnings.iter().any(|w| w.contains("keeping defaults")));
    }

    #[test]
    fn reserved_ctrl_c_is_rejected_with_warning() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.select]
edit = ["ctrl+c"]
"#,
        );
        // Ctrl+C stays unbindable; edit keeps its default.
        assert_eq!(
            keymap.select.action(&KeyChord::parse("ctrl+e").unwrap()),
            Some(SelectAction::Edit)
        );
        assert!(warnings.iter().any(|w| w.contains("reserved")));
    }

    #[test]
    fn duplicate_chord_in_one_context_keeps_earlier_action() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.fuzzy]
move_up = ["enter", "ctrl+p"]
"#,
        );
        // `enter` is owned by `accept` (earlier in precedence order).
        assert_eq!(
            keymap.fuzzy.action(&KeyChord::parse("enter").unwrap()),
            Some(FuzzyAction::Accept)
        );
        assert_eq!(
            keymap.fuzzy.action(&KeyChord::parse("ctrl+p").unwrap()),
            Some(FuzzyAction::MoveUp)
        );
        assert!(warnings.iter().any(|w| w.contains("duplicate key `enter`")));
    }

    #[test]
    fn same_chord_allowed_in_different_contexts() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.fuzzy]
page_up = ["ctrl+u"]

[keybinds.execute.browse]
page_up = ["ctrl+u"]
"#,
        );
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        let chord = KeyChord::parse("ctrl+u").unwrap();
        assert_eq!(keymap.fuzzy.action(&chord), Some(FuzzyAction::PageUp));
        assert_eq!(keymap.browse.action(&chord), Some(BrowseAction::PageUp));
    }

    #[test]
    fn unknown_context_action_and_section_warn_without_failing() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.wat]
whatever = ["x"]

[keybinds.execute.wat]
foo = ["x"]

[keybinds.execute.fuzzy]
frobnicate = ["x"]
accept = ["ctrl+a"]
"#,
        );
        assert!(warnings.iter().any(|w| w.contains("unknown section")));
        assert!(warnings.iter().any(|w| w.contains("unknown context")));
        assert!(warnings.iter().any(|w| w.contains("unknown action")));
        // The valid binding in the same file still applies.
        assert_eq!(
            keymap.fuzzy.action(&KeyChord::parse("ctrl+a").unwrap()),
            Some(FuzzyAction::Accept)
        );
    }

    #[test]
    fn wrong_value_type_warns_and_keeps_defaults() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.fuzzy]
accept = "enter"
"#,
        );
        assert!(warnings.iter().any(|w| w.contains("array of key strings")));
        assert_eq!(
            keymap.fuzzy.action(&KeyChord::parse("enter").unwrap()),
            Some(FuzzyAction::Accept)
        );
    }

    #[test]
    fn plan_example_config_resolves_without_warnings() {
        let (keymap, warnings) = resolve_toml(
            r#"
[keybinds.execute.select]
cycle_mode = ["ctrl+t", "f2"]
edit = ["ctrl+e"]
preview_down = ["ctrl+j", "ctrl+down"]
preview_up = ["ctrl+k", "ctrl+up"]
cancel_or_back = ["esc"]

[keybinds.execute.fuzzy]
accept = ["enter"]
move_up = ["up"]
move_down = ["down"]
page_up = ["pageup"]
page_down = ["pagedown"]

[keybinds.execute.prompt]
accept = ["enter"]
complete_or_next = ["tab"]
previous_variable = ["shift+tab", "backtab"]
literal_newline = ["alt+enter", "ctrl+j"]
return_to_picker = ["esc"]
"#,
        );
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
        assert_eq!(keymap, ExecuteKeymap::default());
    }

    #[test]
    fn space_and_plus_named_keys_parse_display_and_round_trip() {
        assert_eq!(
            KeyChord::parse("space").unwrap(),
            KeyChord::new(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyChord::parse("plus").unwrap(),
            KeyChord::new(KeyCode::Char('+'), KeyModifiers::NONE)
        );
        assert_eq!(KeyChord::parse("space").unwrap().to_string(), "space");
        assert_eq!(KeyChord::parse("plus").unwrap().to_string(), "plus");
        assert_eq!(
            KeyChord::parse("ctrl+plus").unwrap(),
            KeyChord::new(KeyCode::Char('+'), KeyModifiers::CONTROL)
        );
        for raw in ["space", "plus", "ctrl+space", "alt+plus"] {
            let chord = KeyChord::parse(raw).unwrap();
            assert_eq!(KeyChord::parse(&chord.to_string()).unwrap(), chord);
        }
        // Bare `+` still fails to parse (empty base token); `plus` covers it.
        assert!(KeyChord::parse("+").is_err());
        // Single printable chars used by the tuner defaults.
        assert_eq!(
            KeyChord::parse("-").unwrap(),
            KeyChord::new(KeyCode::Char('-'), KeyModifiers::NONE)
        );
        assert_eq!(
            KeyChord::parse("=").unwrap(),
            KeyChord::new(KeyCode::Char('='), KeyModifiers::NONE)
        );
    }

    #[test]
    fn settings_and_new_defaults_resolve() {
        let keymaps = Keymaps::resolve(None);
        assert!(keymaps.warnings.is_empty());
        // Settings: shift+q canonicalizes to `Q`, so both spellings quit.
        let shift_q = KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT);
        assert_eq!(
            keymaps.settings.global.action_for(&shift_q),
            Some(SettingsGlobalAction::Quit)
        );
        assert_eq!(
            keymaps
                .settings
                .global
                .action(&KeyChord::parse("q").unwrap()),
            Some(SettingsGlobalAction::Quit)
        );
        assert_eq!(
            keymaps.settings.list.action(&KeyChord::parse("k").unwrap()),
            Some(SettingsListAction::MoveUp)
        );
        assert_eq!(
            keymaps
                .settings
                .search
                .action(&KeyChord::parse("shift+r").unwrap()),
            Some(SettingsSearchAction::Reset)
        );
        assert_eq!(
            keymaps
                .settings
                .tuner
                .action(&KeyChord::parse("plus").unwrap()),
            Some(SettingsTunerAction::Increase)
        );
        assert_eq!(
            keymaps
                .settings
                .tuner
                .action(&KeyChord::parse("-").unwrap()),
            Some(SettingsTunerAction::Decrease)
        );
        // New: space toggles, letters navigate/act.
        assert_eq!(
            keymaps
                .new
                .confirm_tokens
                .action(&KeyChord::parse("space").unwrap()),
            Some(NewConfirmTokensAction::ToggleVariable)
        );
        assert_eq!(
            keymaps.new.picker.action(&KeyChord::parse("j").unwrap()),
            Some(NewPickerAction::MoveDown)
        );
        assert_eq!(
            keymaps
                .new
                .confirm_name
                .action(&KeyChord::parse("tab").unwrap()),
            Some(NewConfirmNameAction::CompleteOrFocusTokens)
        );
        assert_eq!(
            keymaps
                .new
                .confirm_rename
                .action(&KeyChord::parse("enter").unwrap()),
            Some(NewConfirmRenameAction::Accept)
        );
    }

    #[test]
    fn per_section_remap_and_cross_section_chord_reuse() {
        let value: toml::Value = toml::from_str(
            r#"
[keybinds.execute.fuzzy]
page_up = ["ctrl+u"]

[keybinds.settings.tuner]
increase = ["ctrl+u"]

[keybinds.new.picker]
move_up = ["ctrl+u"]
"#,
        )
        .unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        assert!(
            keymaps.warnings.is_empty(),
            "unexpected warnings: {:?}",
            keymaps.warnings
        );
        let chord = KeyChord::parse("ctrl+u").unwrap();
        assert_eq!(
            keymaps.execute.fuzzy.action(&chord),
            Some(FuzzyAction::PageUp)
        );
        assert_eq!(
            keymaps.settings.tuner.action(&chord),
            Some(SettingsTunerAction::Increase)
        );
        assert_eq!(
            keymaps.new.picker.action(&chord),
            Some(NewPickerAction::MoveUp)
        );
        // Replaced defaults are gone.
        assert_eq!(
            keymaps.new.picker.action(&KeyChord::parse("up").unwrap()),
            None
        );
        // Untouched sections keep defaults.
        assert_eq!(
            keymaps
                .settings
                .list
                .action(&KeyChord::parse("enter").unwrap()),
            Some(SettingsListAction::Select)
        );
    }

    #[test]
    fn conflict_precedence_inside_new_contexts() {
        let value: toml::Value = toml::from_str(
            r#"
[keybinds.new.confirm_tokens]
cancel = ["x"]
back = ["x"]

[keybinds.settings.list]
back = ["f5"]
select = ["f5"]
"#,
        )
        .unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        // Earlier action in precedence order keeps the duplicated chord.
        assert_eq!(
            keymaps
                .new
                .confirm_tokens
                .action(&KeyChord::parse("x").unwrap()),
            Some(NewConfirmTokensAction::Cancel)
        );
        assert_eq!(
            keymaps
                .settings
                .list
                .action(&KeyChord::parse("f5").unwrap()),
            Some(SettingsListAction::Back)
        );
        assert_eq!(
            keymaps
                .warnings
                .iter()
                .filter(|w| w.contains("duplicate key"))
                .count(),
            2
        );
    }

    #[test]
    fn unknown_settings_and_new_contexts_warn() {
        let value: toml::Value = toml::from_str(
            r#"
[keybinds.settings.wat]
foo = ["x"]

[keybinds.new.wat]
foo = ["x"]
"#,
        )
        .unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        assert!(
            keymaps
                .warnings
                .iter()
                .any(|w| w.contains("unknown context `settings.wat`"))
        );
        assert!(
            keymaps
                .warnings
                .iter()
                .any(|w| w.contains("unknown context `new.wat`"))
        );
    }

    #[test]
    fn resolve_text_entry_rule_per_widget_kind() {
        let bindings: ContextBindings<NewPickerAction> = ContextBindings::default();
        let plain_k = KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE);
        let esc = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        // No text entry: plain letters resolve unconditionally.
        assert_eq!(
            bindings.resolve(&plain_k, TextEntry::None),
            Some(NewPickerAction::MoveUp)
        );

        // Conditional text entry: letters resolve only while input is empty.
        assert_eq!(
            bindings.resolve(&plain_k, TextEntry::WhenEmpty(true)),
            Some(NewPickerAction::MoveUp)
        );
        assert_eq!(
            bindings.resolve(&plain_k, TextEntry::WhenEmpty(false)),
            None
        );
        // Named keys and modified chords always resolve.
        assert_eq!(
            bindings.resolve(&esc, TextEntry::WhenEmpty(false)),
            Some(NewPickerAction::CancelOrBack)
        );

        // Always-text widgets: plain printable chords are inert by design.
        assert_eq!(bindings.resolve(&plain_k, TextEntry::Always), None);
        assert_eq!(
            bindings.resolve(&esc, TextEntry::Always),
            Some(NewPickerAction::CancelOrBack)
        );
        // A shifted letter canonicalizes to an uppercase char and stays text.
        let shift_k = KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT);
        let mut remapped: ContextBindings<NewPickerAction> = ContextBindings::default();
        let table: toml::value::Table = toml::from_str(r#"move_up = ["shift+k"]"#).unwrap();
        remapped.apply("new", &table, &mut Vec::new());
        assert_eq!(remapped.resolve(&shift_k, TextEntry::Always), None);
        assert_eq!(
            remapped.resolve(&shift_k, TextEntry::None),
            Some(NewPickerAction::MoveUp)
        );
    }

    #[test]
    fn documented_settings_and_new_defaults_resolve_without_warnings() {
        // Mirrors the commented reference in examples/config.toml: spelling
        // out every default must be a no-op with no warnings.
        let value: toml::Value = toml::from_str(
            r#"
[keybinds.settings.global]
quit = ["q", "shift+q"]

[keybinds.settings.list]
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
select = ["enter"]
reset = ["r", "shift+r"]

[keybinds.settings.search]
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
select = ["enter"]
reset = ["r", "shift+r"]

[keybinds.settings.tuner]
back = ["esc", "backspace"]
move_up = ["up", "k"]
move_down = ["down", "j"]
decrease = ["left", "-"]
increase = ["right", "plus", "="]
reset = ["r", "shift+r"]
save = ["enter"]

[keybinds.new.picker]
accept = ["enter"]
cancel_or_back = ["esc"]
move_up = ["up", "k"]
move_down = ["down", "j"]
backspace = ["backspace"]

[keybinds.new.confirm_name]
accept = ["enter"]
cancel = ["esc"]
complete_or_focus_tokens = ["tab", "down"]
backspace = ["backspace"]

[keybinds.new.confirm_tokens]
cancel = ["esc"]
back = ["b"]
move_up = ["up", "k"]
move_down = ["down", "j"]
toggle_variable = ["space"]
rename = ["e"]
edit_name = ["n"]
accept = ["enter"]

[keybinds.new.confirm_rename]
cancel = ["esc"]
accept = ["enter"]
backspace = ["backspace"]
"#,
        )
        .unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        assert!(
            keymaps.warnings.is_empty(),
            "unexpected warnings: {:?}",
            keymaps.warnings
        );
        assert_eq!(keymaps.settings, SettingsKeymap::default());
        assert_eq!(keymaps.new, NewKeymap::default());
    }

    #[test]
    fn emergency_cancel_detection_is_unconditional() {
        assert!(ExecuteKeymap::is_emergency_cancel(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(!ExecuteKeymap::is_emergency_cancel(&KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE
        )));
    }
}
