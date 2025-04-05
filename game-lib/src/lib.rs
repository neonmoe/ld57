#![no_std]

use engine::Engine;
use platform::{Instant, Platform};

pub struct Game {}

impl Game {
    pub fn new() -> Game {
        Game {}
    }

    pub fn iterate(&mut self, _engine: &mut Engine, _platform: &dyn Platform, _timestamp: Instant) {
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}
