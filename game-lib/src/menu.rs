use arrayvec::ArrayVec;
use engine::input::InputDeviceState;

use crate::{Button, Sprite, game_object::JobStationVariant};

pub enum MenuMode {
    MenuStack(ArrayVec<Menu, 3>),
    BuildPlacement,
}

#[derive(Clone, Copy)]
pub enum MenuAction {
    Select,
    Next,
    Previous,
}

pub struct Menu {
    entries: ArrayVec<MenuEntry, 8>,
    selected_index: usize,
    pub rendered: bool,
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
            rendered: true,
        }
    }

    pub fn options(flip_accept_cancel: bool) -> Menu {
        let mut entries = ArrayVec::new();
        entries.push(MenuEntry::Volume);
        entries.push(MenuEntry::FlipAcceptCancel(flip_accept_cancel));
        Menu {
            entries,
            selected_index: 0,
            rendered: true,
        }
    }

    pub fn manage_characters(character_count: usize) -> Menu {
        let mut entries = ArrayVec::new();
        for brain_index in 0..character_count.min(entries.capacity()) {
            entries.push(MenuEntry::ManageCharacter { brain_index });
        }
        Menu {
            entries,
            selected_index: 0,
            rendered: false,
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn hover_index(&self) -> usize {
        self.selected_index
    }

    pub fn hover_entry(&self) -> MenuEntry {
        self.entries[self.selected_index]
    }

    pub fn entry(&self, index: usize) -> &MenuEntry {
        &self.entries[index]
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
    ) -> Option<(&mut MenuEntry, MenuAction)> {
        if input.actions[Button::Up as usize].pressed {
            self.selected_index = self.selected_index.saturating_sub(1);
        }
        if input.actions[Button::Down as usize].pressed {
            self.selected_index = (self.selected_index + 1).min(self.entries.len() - 1);
        }
        if input.actions[Button::Accept as usize].pressed {
            return Some((&mut self.entries[self.selected_index], MenuAction::Select));
        } else if input.actions[Button::Left as usize].pressed {
            return Some((&mut self.entries[self.selected_index], MenuAction::Previous));
        } else if input.actions[Button::Right as usize].pressed {
            return Some((&mut self.entries[self.selected_index], MenuAction::Next));
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
    BuildSelect(JobStationVariant),
    ManageCharacters,
    ManageCharacter { brain_index: usize },
    Volume,
    FlipAcceptCancel(bool),
}

impl MenuEntry {
    fn sprite(self) -> Option<Sprite> {
        match self {
            MenuEntry::Quit => Some(Sprite::MenuItemQuit),
            MenuEntry::Continue => Some(Sprite::MenuItemContinue),
            MenuEntry::Options => Some(Sprite::MenuItemOptions),
            MenuEntry::Build => Some(Sprite::MenuItemBuild),
            MenuEntry::BuildSelect(_) => None,
            MenuEntry::ManageCharacters => Some(Sprite::MenuItemManageChars),
            MenuEntry::ManageCharacter { .. } => None,
            MenuEntry::Volume => Some(Sprite::MenuItemVolume),
            MenuEntry::FlipAcceptCancel(true) => Some(Sprite::MenuItemFlipACtrue),
            MenuEntry::FlipAcceptCancel(false) => Some(Sprite::MenuItemFlipACfalse),
        }
    }
}
