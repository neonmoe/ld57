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

const BIT_GRID_BITS: usize = 128;

pub struct BitGrid<'a> {
    values: FixedVec<'a, u128>,
    width: usize,
    height: usize,
    stride: usize,
}

impl BitGrid<'_> {
    pub fn new<'a>(
        arena: &'a LinearAllocator,
        (width, height): (usize, usize),
    ) -> Option<BitGrid<'a>> {
        let stride = width.div_ceil(BIT_GRID_BITS);
        let mut values = FixedVec::new(arena, stride * height)?;
        values.fill_with_zeroes();
        Some(BitGrid {
            values,
            width,
            height,
            stride,
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

    pub fn set(&mut self, pos: TilePosition, new_value: bool) {
        assert!(pos.x >= 0);
        assert!((pos.x as usize) < self.width);
        assert!(pos.y >= 0);
        assert!((pos.y as usize) < self.height);

        let x = pos.x as usize;
        let bitfield_x = x / BIT_GRID_BITS;
        let x_bit_offset = x % BIT_GRID_BITS;
        if new_value {
            self.values[bitfield_x + pos.y as usize * self.stride] |=
                (new_value as u128) << x_bit_offset;
        } else {
            self.values[bitfield_x + pos.y as usize * self.stride] &=
                !((new_value as u128) << x_bit_offset);
        }
    }

    pub fn get(&self, pos: TilePosition) -> bool {
        assert!(pos.x >= 0);
        assert!((pos.x as usize) < self.width);
        assert!(pos.y >= 0);
        assert!((pos.y as usize) < self.height);

        let x = pos.x as usize;
        let bitfield_x = x / BIT_GRID_BITS;
        let bitfield = self.values[bitfield_x + pos.y as usize * self.stride];
        let x_bit_offset = x % BIT_GRID_BITS;
        (bitfield & (1 << x_bit_offset)) != 0
    }
}

#[cfg(test)]
mod tests {
    use engine::{allocators::LinearAllocator, static_allocator};

    use crate::game_object::TilePosition;

    use super::BitGrid;

    #[test]
    fn bit_grid_works() {
        static ARENA: &LinearAllocator = static_allocator!(100000);
        let mut grid = BitGrid::new(ARENA, (150, 150)).unwrap();

        for y in 0..150 {
            for x in 0..150 {
                assert!(!grid.get(TilePosition::new(x, y)));
            }
        }

        grid.set(TilePosition::new(50, 30), true);
        grid.set(TilePosition::new(140, 30), true);

        for y in 0..150 {
            for x in 0..150 {
                let set = (x == 50 || x == 140) && y == 30;
                assert!(
                    grid.get(TilePosition::new(x, y)) == set,
                    "unexpected grid value at {x}, {y}"
                );
            }
        }
    }
}
