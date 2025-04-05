use arrayvec::ArrayVec;
use engine::game_objects::Scene;

use crate::{game_object::TilePosition, tilemap::Tilemap};

pub fn find_path(
    from: TilePosition,
    to: TilePosition,
    tilemap: &Tilemap,
    scene: &mut Scene,
) -> Path {
    let mut path = Path::default();
    path
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    const fn to_u8(self) -> u8 {
        match self {
            Direction::Up => 0b00,
            Direction::Down => 0b01,
            Direction::Left => 0b10,
            Direction::Right => 0b11,
        }
    }

    const fn from_u8(u: u8) -> Direction {
        match u {
            0b00 => Direction::Up,
            0b01 => Direction::Down,
            0b10 => Direction::Left,
            _ => Direction::Right,
        }
    }
}

#[derive(Default, Clone)]
pub struct Path {
    /// Each u8 represents 4 steps, so the maximum length for a path is 480
    /// steps.
    step_quads: ArrayVec<u8, 120>,
    steps_in_last_quad: u8,
}

impl Path {
    /// Adds a step to the end of the path.
    ///
    /// Returns `false` if the Path is full (480 steps is the maximum).
    pub fn add_step(&mut self, direction: Direction) -> bool {
        if self.steps_in_last_quad == 0 {
            if self.step_quads.try_push(direction.to_u8()).is_err() {
                return false;
            }
            self.steps_in_last_quad += 1;
        } else {
            let quad = self.step_quads.last_mut().unwrap();
            *quad |= direction.to_u8() << (self.steps_in_last_quad * 2);
            self.steps_in_last_quad = (self.steps_in_last_quad + 1) % 4;
        }
        true
    }
}

impl IntoIterator for Path {
    type Item = Direction;
    type IntoIter = PathIterator;

    fn into_iter(self) -> Self::IntoIter {
        PathIterator {
            current_quad_step_offset: 0,
            current_quad_index: 0,
            steps_in_last_quad: self.steps_in_last_quad,
            step_quads: self.step_quads.clone(),
        }
    }
}

pub struct PathIterator {
    current_quad_step_offset: u8,
    current_quad_index: u8,
    steps_in_last_quad: u8,
    step_quads: ArrayVec<u8, 120>,
}

impl Iterator for PathIterator {
    type Item = Direction;

    fn next(&mut self) -> Option<Self::Item> {
        let idx = self.current_quad_index as usize;
        if idx >= self.step_quads.len() {
            return None;
        }

        let steps_in_current_quad = if idx == self.step_quads.len() - 1 {
            self.steps_in_last_quad
        } else {
            4
        };

        let current_quad = self.step_quads[idx];
        let direction =
            Direction::from_u8((current_quad >> (self.current_quad_step_offset * 2)) & 0b11);

        self.current_quad_step_offset += 1;
        if self.current_quad_step_offset == steps_in_current_quad {
            self.current_quad_step_offset = 0;
            self.current_quad_index += 1;
        }

        Some(direction)
    }
}
