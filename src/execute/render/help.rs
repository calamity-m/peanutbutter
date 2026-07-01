//! Footer help derivation for the execute UI.
//!
//! Help lines are built from the active keymap rather than fixed strings so
//! remapped keys are what the UI teaches. Intentionally unbound actions are
//! omitted rather than shown with stale defaults.

use crate::keybinds::{BrowseAction, FuzzyAction, PromptAction, SelectAction, TagsAction};

use super::super::app::{ExecutionApp, NavigationMode, SuggestionProvider};

impl<P: SuggestionProvider> ExecutionApp<P> {
    /// Derives footer help for the select screen from the active keymap.
    pub(super) fn select_help(&self, selected_is_dir: bool) -> String {
        let select = &self.keymap.select;
        let preview = match (
            select.hint(SelectAction::PreviewDown),
            select.hint(SelectAction::PreviewUp),
        ) {
            (Some(down), Some(up)) => Some(format!("{down}/{up}")),
            (only, None) => only,
            (None, only) => only,
        };
        let cycle = select.hint(SelectAction::CycleMode);
        let cancel = select.hint(SelectAction::CancelOrBack);
        let edit = select.hint(SelectAction::Edit);
        let items: Vec<(Option<String>, &str)> = match self.nav_mode {
            NavigationMode::Fuzzy => vec![
                (self.keymap.fuzzy.hint(FuzzyAction::Accept), "accept"),
                (edit, "edit"),
                (preview, "preview"),
                (cycle, "browse"),
                (cancel, "cancel"),
            ],
            NavigationMode::Browse => {
                let accept = self.keymap.browse.hint(BrowseAction::AcceptOrOpen);
                let complete = self.keymap.browse.hint(BrowseAction::Complete);
                if selected_is_dir {
                    vec![
                        (complete, "complete"),
                        (accept, "open"),
                        (preview, "preview"),
                        (cycle, "tags"),
                        (cancel, "cancel"),
                    ]
                } else {
                    vec![
                        (complete, "complete"),
                        (accept, "accept"),
                        (edit, "edit"),
                        (preview, "preview"),
                        (cycle, "tags"),
                        (cancel, "cancel"),
                    ]
                }
            }
            NavigationMode::Tags => {
                let accept = self.keymap.tags.hint(TagsAction::AcceptOrDrill);
                if self.tags.drill().is_some() {
                    vec![
                        (Some("type".to_string()), "filter"),
                        (accept, "accept"),
                        (self.keymap.tags.hint(TagsAction::ReturnToTags), "tags"),
                        (self.keymap.tags.hint(TagsAction::Backspace), "clear/back"),
                        (cycle, "search"),
                    ]
                } else {
                    vec![
                        (Some("type".to_string()), "filter"),
                        (accept, "open"),
                        (preview, "preview"),
                        (cycle, "search"),
                        (cancel, "cancel"),
                    ]
                }
            }
        };
        help_text(items)
    }

    /// Derives footer help for the variable prompt screen.
    pub(super) fn prompt_help(&self) -> String {
        let keymap = &self.keymap.prompt;
        help_text(vec![
            (keymap.hint(PromptAction::CompleteOrNext), "complete/next"),
            (keymap.hint(PromptAction::PreviousVariable), "prev"),
            (keymap.hint(PromptAction::Accept), "accept"),
            (keymap.hint(PromptAction::ReturnToPicker), "return"),
        ])
    }
}

/// Formats `(display chord, label)` pairs into one footer help line, dropping
/// entries whose action is unbound.
fn help_text(items: Vec<(Option<String>, &str)>) -> String {
    items
        .into_iter()
        .filter_map(|(chord, label)| chord.map(|chord| format!("{chord} {label}")))
        .collect::<Vec<_>>()
        .join("  ")
}
