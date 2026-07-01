//! Customizable keybinds for the `pb execute` TUI.
//!
//! Users remap actions through `[keybinds.execute.<context>]` TOML tables;
//! each action maps to an array of key chord strings. Omitted actions keep
//! their defaults, an empty array unbinds the action, and invalid chords are
//! collected as non-fatal warnings so the rest of the config still applies.
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

    /// Apply one `[keybinds.execute.<context>]` table. Invalid entries become
    /// warnings; valid entries replace that action's defaults; an empty array
    /// unbinds the action.
    fn apply(&mut self, table: &toml::value::Table, warnings: &mut Vec<String>) {
        for (name, value) in table {
            let Some(action) = A::ALL.iter().find(|action| action.name() == name) else {
                warnings.push(format!(
                    "keybinds: unknown action `{name}` in [keybinds.execute.{}]",
                    A::CONTEXT
                ));
                continue;
            };
            let Some(raw_chords) = value.as_array() else {
                warnings.push(format!(
                    "keybinds: expected an array of key strings for execute.{}.{name}",
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
                        "keybinds: expected a key string for execute.{}.{name}, got {raw}",
                        A::CONTEXT
                    ));
                    continue;
                };
                match KeyChord::parse(raw) {
                    Ok(chord) if chord == KeyChord::reserved_cancel() => {
                        warnings.push(format!(
                            "keybinds: `{raw}` is reserved for emergency cancel and cannot be bound (execute.{}.{name})",
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
                            "keybinds: invalid key for execute.{}.{name}: {err}",
                            A::CONTEXT
                        ));
                    }
                }
            }
            if chords.is_empty() {
                warnings.push(format!(
                    "keybinds: no valid keys configured for execute.{}.{name}; keeping defaults",
                    A::CONTEXT
                ));
            } else {
                self.bindings[Self::index_of(*action)] = chords;
            }
        }
    }

    /// Drop chords already owned by an earlier action in precedence order,
    /// warning about each ignored duplicate.
    fn resolve_conflicts(&mut self, warnings: &mut Vec<String>) {
        let mut seen: Vec<(KeyChord, &'static str)> = Vec::new();
        for (action, chords) in A::ALL.iter().zip(&mut self.bindings) {
            chords.retain(|chord| {
                if let Some((_, owner)) = seen.iter().find(|(c, _)| c == chord) {
                    warnings.push(format!(
                        "keybinds: duplicate key `{chord}` in execute.{} ignored for {} (already bound to {owner})",
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
        KeyChord::from_event(event) == KeyChord::reserved_cancel()
    }

    fn apply_root(&mut self, table: &toml::value::Table, warnings: &mut Vec<String>) {
        for (section, value) in table {
            if section != "execute" {
                warnings.push(format!("keybinds: unknown section `keybinds.{section}`"));
                continue;
            }
            let Some(contexts) = value.as_table() else {
                warnings.push("keybinds: expected [keybinds.execute] to be a table".to_string());
                continue;
            };
            for (context, actions) in contexts {
                let Some(actions) = actions.as_table() else {
                    warnings.push(format!(
                        "keybinds: expected [keybinds.execute.{context}] to be a table"
                    ));
                    continue;
                };
                match context.as_str() {
                    "select" => self.select.apply(actions, warnings),
                    "fuzzy" => self.fuzzy.apply(actions, warnings),
                    "browse" => self.browse.apply(actions, warnings),
                    "tags" => self.tags.apply(actions, warnings),
                    "prompt" => self.prompt.apply(actions, warnings),
                    other => warnings.push(format!(
                        "keybinds: unknown context `execute.{other}`; expected one of: select, fuzzy, browse, tags, prompt"
                    )),
                }
            }
        }
    }
}

/// Every resolved keymap plus the diagnostics produced while resolving them.
///
/// This is the output of interpreting the raw `[keybinds]` config value —
/// per-command keymaps (`execute` today; `settings`/`new` are the intended
/// future siblings) alongside the non-fatal warnings resolution produced.
/// Warnings live here rather than as standalone config because they describe
/// this resolution, and each TUI decides how to surface them (`pb execute`
/// shows them as status so they never touch the stdout command payload).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Keymaps {
    /// Resolved keybinds for the `pb execute` TUI.
    pub execute: ExecuteKeymap,
    /// Non-fatal validation warnings: invalid or reserved chords, unknown
    /// contexts/actions, wrong value types, and ignored duplicate bindings.
    pub warnings: Vec<String>,
}

impl Keymaps {
    /// Resolve the raw `[keybinds]` config value. `None` (no `[keybinds]`
    /// section) yields pure defaults with no warnings.
    pub fn resolve(raw: Option<&toml::Value>) -> Self {
        let mut execute = ExecuteKeymap::default();
        let mut warnings = Vec::new();
        if let Some(raw) = raw {
            match raw.as_table() {
                Some(table) => execute.apply_root(table, &mut warnings),
                None => warnings.push("keybinds: expected [keybinds] to be a table".to_string()),
            }
        }
        execute.select.resolve_conflicts(&mut warnings);
        execute.fuzzy.resolve_conflicts(&mut warnings);
        execute.browse.resolve_conflicts(&mut warnings);
        execute.tags.resolve_conflicts(&mut warnings);
        execute.prompt.resolve_conflicts(&mut warnings);
        Self { execute, warnings }
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
[keybinds.settings]
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
