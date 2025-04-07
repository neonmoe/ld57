use arrayvec::ArrayVec;
use engine::input::InputDeviceState;

use crate::{Button, Sprite};

pub enum MenuMode {
    MenuStack(ArrayVec<Menu, 3>),
    BuildPlacement,
}

pub struct Menu {
    entries: ArrayVec<MenuEntry, 8>,
    selected_index: usize,
}

impl Menu {
    pub fn main_menu() -> Menu {
        let mut entries = ArrayVec::new();
        entries.push(MenuEntry::Continue);
        entries.push(MenuEntry::Build);
        entries.push(MenuEntry::ManageCharacters);
        entries.push(MenuEntry::Options);
        entries.push(MenuEntry::Quit);
        Menu {
            entries,
            selected_index: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn hover_index(&self) -> usize {
        self.selected_index
    }

    pub fn sprite(&self, index: usize) -> Option<Sprite> {
        let entry = self.entries.get(index)?;
        entry.sprite()
    }

    /// Updates the selection based on input, and returns a [`MenuEntry`] if one
    /// was selected.
    pub fn update(
        &mut self,
        input: &InputDeviceState<{ Button::_Count as usize }>,
    ) -> Option<MenuEntry> {
        if input.actions[Button::Up as usize].pressed {
            self.selected_index = self.selected_index.saturating_sub(1);
        }
        if input.actions[Button::Down as usize].pressed {
            self.selected_index = (self.selected_index + 1).min(self.entries.len() - 1);
        }
        if input.actions[Button::Accept as usize].pressed {
            return Some(self.entries[self.selected_index]);
        }
        None
    }
}

#[derive(Clone, Copy)]
pub enum MenuEntry {
    Quit,
    Continue,
    Options,
    Build,
    BuildSelectEnergy,
    BuildSelectOxygen,
    ManageCharacters,
}

impl MenuEntry {
    fn sprite(self) -> Option<Sprite> {
        match self {
            MenuEntry::Quit => Some(Sprite::MenuItemQuit),
            MenuEntry::Continue => Some(Sprite::MenuItemContinue),
            MenuEntry::Options => Some(Sprite::MenuItemOptions),
            MenuEntry::Build => Some(Sprite::MenuItemBuild),
            MenuEntry::BuildSelectEnergy => None,
            MenuEntry::BuildSelectOxygen => None,
            MenuEntry::ManageCharacters => Some(Sprite::MenuItemManageChars),
        }
    }
}
