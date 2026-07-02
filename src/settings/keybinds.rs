//! Type-erased keybind rows for the interactive settings editor.

use crate::keybinds::{
    ContextBindings, ExecuteKeymap, KeyChord, KeymapAction, Keymaps, NewKeymap,
    SettingsGlobalAction, SettingsKeymap,
};

/// One editable keybind action row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KeybindEntry {
    pub(crate) section: &'static str,
    pub(crate) context: &'static str,
    pub(crate) action: &'static str,
    pub(crate) defaults: Vec<KeyChord>,
    pub(crate) original: Vec<KeyChord>,
    pub(crate) current: Vec<KeyChord>,
}

impl KeybindEntry {
    /// Whether the row differs from the last saved/resolved baseline.
    pub(crate) fn changed(&self) -> bool {
        self.current != self.original
    }

    /// Whether the row differs from the built-in default.
    pub(crate) fn differs_from_default(&self) -> bool {
        self.current != self.defaults
    }

    /// Mark the current value as the saved baseline.
    pub(crate) fn accept_current(&mut self) {
        self.original = self.current.clone();
    }
}

fn parsed_defaults<A: KeymapAction>(action: A) -> Vec<KeyChord> {
    let mut chords = action
        .default_keys()
        .iter()
        .map(|raw| KeyChord::parse(raw).expect("default keybinds are valid chord strings"))
        .collect::<Vec<_>>();
    chords.dedup();
    chords
}

fn entries_for<A: KeymapAction>(
    section: &'static str,
    bindings: &ContextBindings<A>,
) -> Vec<KeybindEntry> {
    A::ALL
        .iter()
        .map(|action| {
            let original = bindings.chords(*action).to_vec();
            KeybindEntry {
                section,
                context: A::CONTEXT,
                action: action.name(),
                defaults: parsed_defaults(*action),
                current: original.clone(),
                original,
            }
        })
        .collect()
}

fn execute_entries(keymap: &ExecuteKeymap) -> Vec<KeybindEntry> {
    let mut entries = Vec::new();
    entries.extend(entries_for("execute", &keymap.select));
    entries.extend(entries_for("execute", &keymap.fuzzy));
    entries.extend(entries_for("execute", &keymap.browse));
    entries.extend(entries_for("execute", &keymap.tags));
    entries.extend(entries_for("execute", &keymap.prompt));
    entries
}

fn settings_entries(keymap: &SettingsKeymap) -> Vec<KeybindEntry> {
    let mut entries = Vec::new();
    entries.extend(entries_for("settings", &keymap.global));
    entries.extend(entries_for("settings", &keymap.list));
    entries.extend(entries_for("settings", &keymap.search));
    entries.extend(entries_for("settings", &keymap.tuner));
    entries.extend(entries_for("settings", &keymap.keybinds));
    entries
}

fn new_entries(keymap: &NewKeymap) -> Vec<KeybindEntry> {
    let mut entries = Vec::new();
    entries.extend(entries_for("new", &keymap.picker));
    entries.extend(entries_for("new", &keymap.confirm_name));
    entries.extend(entries_for("new", &keymap.confirm_tokens));
    entries.extend(entries_for("new", &keymap.confirm_rename));
    entries
}

/// Build editable rows for every configurable keybind in all interactive commands.
pub(crate) fn entries_from_keymaps(keymaps: &Keymaps) -> Vec<KeybindEntry> {
    let mut entries = Vec::new();
    entries.extend(execute_entries(&keymaps.execute));
    entries.extend(settings_entries(&keymaps.settings));
    entries.extend(new_entries(&keymaps.new));
    entries
}

/// The editable commands in display order.
pub(crate) const COMMANDS: &[&str] = &["execute", "settings", "new"];

/// Find a conflicting owner for `chord` within the target context.
pub(crate) fn conflict_owner<'a>(
    entries: &'a [KeybindEntry],
    target_idx: usize,
    chord: &KeyChord,
) -> Option<&'a KeybindEntry> {
    let target = entries.get(target_idx)?;
    entries.iter().enumerate().find_map(|(idx, entry)| {
        (idx != target_idx
            && entry.section == target.section
            && entry.context == target.context
            && entry.current.contains(chord))
        .then_some(entry)
    })
}

/// Find a settings.global binding that would shadow a settings-context binding.
pub(crate) fn settings_global_shadow<'a>(
    entries: &'a [KeybindEntry],
    target_idx: usize,
    chord: &KeyChord,
) -> Option<&'a KeybindEntry> {
    let target = entries.get(target_idx)?;
    if target.section != "settings" {
        return None;
    }
    if target.context == SettingsGlobalAction::CONTEXT {
        return entries.iter().find(|entry| {
            entry.section == "settings"
                && entry.context != SettingsGlobalAction::CONTEXT
                && entry.current.contains(chord)
        });
    }
    entries.iter().find(|entry| {
        entry.section == "settings"
            && entry.context == SettingsGlobalAction::CONTEXT
            && entry.current.contains(chord)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinds::{
        BrowseAction, FuzzyAction, NewConfirmNameAction, NewConfirmRenameAction,
        NewConfirmTokensAction, NewPickerAction, PromptAction, SelectAction,
        SettingsKeybindsAction, SettingsListAction, SettingsSearchAction, SettingsTunerAction,
        TagsAction,
    };

    #[test]
    fn builds_rows_from_macro_tables_and_resolved_chords() {
        let value: toml::Value = toml::from_str(
            r#"
[keybinds.execute.fuzzy]
accept = ["ctrl+x"]
[keybinds.settings.keybinds]
save = ["ctrl+s"]
"#,
        )
        .unwrap();
        let keymaps = Keymaps::resolve(value.get("keybinds"));
        assert!(keymaps.warnings.is_empty(), "{:?}", keymaps.warnings);
        let entries = entries_from_keymaps(&keymaps);
        let expected = SelectAction::ALL.len()
            + FuzzyAction::ALL.len()
            + BrowseAction::ALL.len()
            + TagsAction::ALL.len()
            + PromptAction::ALL.len()
            + SettingsGlobalAction::ALL.len()
            + SettingsListAction::ALL.len()
            + SettingsSearchAction::ALL.len()
            + SettingsTunerAction::ALL.len()
            + SettingsKeybindsAction::ALL.len()
            + NewPickerAction::ALL.len()
            + NewConfirmNameAction::ALL.len()
            + NewConfirmTokensAction::ALL.len()
            + NewConfirmRenameAction::ALL.len();
        assert_eq!(entries.len(), expected);
        let accept = entries
            .iter()
            .find(|e| (e.section, e.context, e.action) == ("execute", "fuzzy", "accept"))
            .unwrap();
        assert_eq!(accept.original, vec![KeyChord::parse("ctrl+x").unwrap()]);
        assert_eq!(accept.current, accept.original);
        assert_eq!(accept.defaults, vec![KeyChord::parse("enter").unwrap()]);
    }
}
