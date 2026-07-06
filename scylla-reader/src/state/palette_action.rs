use crate::messenger::AppCommand;
use crate::state::AppState;
use std::sync::mpsc;

#[derive(Debug, Clone)]
pub struct PaletteAction {
    pub category: &'static str,
    pub label: &'static str,
    pub keys: &'static str,
    pub handler: fn(&mut AppState, &mpsc::Sender<AppCommand>),
}

impl PartialEq for PaletteAction {
    fn eq(&self, other: &Self) -> bool {
        self.category == other.category && self.label == other.label && self.keys == other.keys
    }
}
