use core::ops::{Index, IndexMut};

use bytemuck::Zeroable;
use engine::{allocators::LinearAllocator, collections::FixedVec};

use crate::game_object::TilePosition;

pub struct Grid<'a, T> {
    values: FixedVec<'a, T>,
    width: usize,
    height: usize,
}

impl<T: Zeroable> Grid<'_, T> {
    pub fn new_zeroed<'a>(
        arena: &'a LinearAllocator,
        (width, height): (usize, usize),
    ) -> Option<Grid<'a, T>> {
        let mut values = FixedVec::new(arena, width * height)?;
        values.fill_with_zeroes();
        Some(Grid {
            values,
            width,
            height,
        })
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub const fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    pub const fn in_bounds(&self, pos: TilePosition) -> bool {
        pos.0.x >= 0
            && pos.0.y >= 0
            && (pos.0.x as usize) < self.width
            && (pos.0.y as usize) < self.height
    }
}

impl<T> Index<TilePosition> for Grid<'_, T> {
    type Output = T;

    fn index(&self, index: TilePosition) -> &Self::Output {
        &self.values[index.x as usize + index.y as usize * self.width]
    }
}

impl<T> IndexMut<TilePosition> for Grid<'_, T> {
    fn index_mut(&mut self, index: TilePosition) -> &mut Self::Output {
        &mut self.values[index.x as usize + index.y as usize * self.width]
    }
}

impl<T> Index<(usize, usize)> for Grid<'_, T> {
    type Output = T;

    fn index(&self, index: (usize, usize)) -> &Self::Output {
        &self.values[index.0 + index.1 * self.width]
    }
}

impl<T> IndexMut<(usize, usize)> for Grid<'_, T> {
    fn index_mut(&mut self, index: (usize, usize)) -> &mut Self::Output {
        &mut self.values[index.0 + index.1 * self.width]
    }
}
