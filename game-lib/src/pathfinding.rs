use core::{
    fmt::{self, Debug},
    ops::{Add, Neg},
};

use arrayvec::ArrayVec;
use bytemuck::Zeroable;
use engine::{allocators::LinearAllocator, collections::FixedVec};
use glam::I16Vec2;

use crate::{
    game_object::TilePosition,
    grid::{BitGrid, Grid},
};

pub fn find_path_to(
    from: TilePosition,
    to: TilePosition,
    walls: &BitGrid,
    temp_arena: &LinearAllocator,
) -> Option<Path> {
    let mut destinations = BitGrid::new(temp_arena, walls.size())?;
    destinations.set(to, true);
    find_path_to_any(from, &destinations, walls, temp_arena)
}

pub fn find_path_to_any(
    from: TilePosition,
    destinations: &BitGrid,
    walls: &BitGrid,
    temp_arena: &LinearAllocator,
) -> Option<Path> {
    let mut try_positions: FixedVec<TilePosition> =
        FixedVec::new(temp_arena, walls.width() * walls.height())?;
    let mut shortest_distance_to_pos: Grid<u8> = Grid::new_zeroed(temp_arena, walls.size())?;
    let mut step_to_previous_in_path: Grid<Direction> = Grid::new_zeroed(temp_arena, walls.size())?;

    let _ = try_positions.push(from);
    while !try_positions.is_empty() {
        let mut try_pos_index = 0;
        let mut try_pos = try_positions[try_pos_index];
        let mut try_pos_distance = shortest_distance_to_pos[try_pos];
        for (i, other) in try_positions.iter().enumerate().skip(1) {
            let dist = shortest_distance_to_pos[*other];
            if dist < try_pos_distance {
                try_pos_distance = dist;
                try_pos = *other;
                try_pos_index = i;
            }
        }

        // Backtrack and finish if this is a valid destination (and walkable).
        if destinations.get(try_pos) && !walls.get(try_pos) {
            let mut path_to_start = Path::default();
            let mut path_end = try_pos;
            while path_end != from && !path_to_start.is_full() {
                let dir = step_to_previous_in_path[path_end];
                path_end = path_end + dir;
                path_to_start.add_step(dir);
            }
            if path_end == from {
                return Some(path_to_start.reverse());
            } else {
                return None;
            }
        }

        // Otherwise, remove this position from and add walkable neighbors to
        // the try list.
        let last_index = try_positions.len() - 1;
        try_positions.swap(try_pos_index, last_index);
        try_positions.truncate(last_index);
        for dir in Direction::ALL_DIRECTIONS {
            if step_to_previous_in_path[try_pos] == dir {
                continue;
            }

            let neighbor = try_pos + dir;
            if walls.in_bounds(neighbor)
                && !walls.get(neighbor)
                && shortest_distance_to_pos[neighbor] == 0
            {
                let could_add_neighbor = try_positions.push(neighbor);
                debug_assert!(could_add_neighbor.is_ok());
                shortest_distance_to_pos[neighbor] = try_pos_distance + 1;
                step_to_previous_in_path[neighbor] = -dir;
            }
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Zeroable)]
#[repr(u8)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub const ALL_DIRECTIONS: [Direction; 4] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ];

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

impl Neg for Direction {
    type Output = Direction;

    fn neg(self) -> Self::Output {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }
}

impl Add<Direction> for TilePosition {
    type Output = TilePosition;

    fn add(self, rhs: Direction) -> Self::Output {
        match rhs {
            Direction::Up => TilePosition(self.0.add(I16Vec2::new(0, -1))),
            Direction::Down => TilePosition(self.0.add(I16Vec2::new(0, 1))),
            Direction::Left => TilePosition(self.0.add(I16Vec2::new(-1, 0))),
            Direction::Right => TilePosition(self.0.add(I16Vec2::new(1, 0))),
        }
    }
}

#[derive(Default, Clone)]
pub struct Path {
    /// Each u8 represents 4 steps, so the maximum length for a path is 224
    /// steps.
    step_quads: ArrayVec<u8, 56>,
    steps_in_last_quad: u8,
}

impl Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = &mut f.debug_list();
        for step in self {
            match step {
                Direction::Up => list = list.entry(&'↑'),
                Direction::Down => list = list.entry(&'↓'),
                Direction::Left => list = list.entry(&'←'),
                Direction::Right => list = list.entry(&'→'),
            }
        }
        list.finish()
    }
}

impl Path {
    /// Adds a step to the end of the path.
    ///
    /// Returns `false` if the Path is full (480 steps is the maximum).
    pub fn add_step(&mut self, direction: Direction) -> bool {
        if self.steps_in_last_quad % 4 == 0 {
            if self.step_quads.try_push(direction.to_u8()).is_err() {
                return false;
            }
            self.steps_in_last_quad = (self.steps_in_last_quad + 1) % 4;
        } else {
            let quad = self.step_quads.last_mut().unwrap();
            *quad |= direction.to_u8() << (self.steps_in_last_quad * 2);
            self.steps_in_last_quad += 1;
        }
        true
    }

    /// Removes the latest step from the end of the path, if it's not empty.
    pub fn pop_step(&mut self) {
        if self.is_empty() {
            return;
        }
        self.steps_in_last_quad -= 1;
        if self.steps_in_last_quad == 0 {
            self.step_quads.pop();
            if !self.step_quads.is_empty() {
                self.steps_in_last_quad = 4;
            }
        }
    }

    pub fn reverse(&self) -> Path {
        let mut path = Path::default();
        for step in self.into_iter().rev() {
            path.add_step(-step);
        }
        path
    }

    pub fn is_empty(&self) -> bool {
        self.step_quads.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.steps_in_last_quad == 4 && self.step_quads.is_full()
    }

    pub fn len(&self) -> u8 {
        self.steps_in_last_quad + (self.step_quads.len() as u8).saturating_sub(1) * 4
    }
}

impl IntoIterator for &Path {
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
    step_quads: ArrayVec<u8, 56>,
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

impl DoubleEndedIterator for PathIterator {
    fn next_back(&mut self) -> Option<Self::Item> {
        let current_quad = self.step_quads.last()?;
        let last_quad_offset = self.steps_in_last_quad - 1;
        let direction = Direction::from_u8((current_quad >> (last_quad_offset * 2)) & 0b11);

        self.steps_in_last_quad -= 1;
        if self.steps_in_last_quad == 0 {
            let more_steps_left = self.step_quads.pop().is_some();
            if more_steps_left {
                self.steps_in_last_quad = 4;
            }
        }

        Some(direction)
    }
}

#[cfg(test)]
mod tests {
    use engine::{allocators::LinearAllocator, static_allocator};

    use crate::{
        game_object::TilePosition,
        grid::BitGrid,
        pathfinding::{Direction, Path, find_path_to},
    };

    #[test]
    pub fn pathfinding_seems_to_work() {
        // The map (start is @, end is *, . is walkable):
        // . . . . .
        // @ # . # #
        // . # . # *
        // . . . . .
        let mut expected_path = Path::default();
        expected_path.add_step(Direction::Down);
        expected_path.add_step(Direction::Down);
        expected_path.add_step(Direction::Right);
        expected_path.add_step(Direction::Right);
        expected_path.add_step(Direction::Right);
        expected_path.add_step(Direction::Right);
        expected_path.add_step(Direction::Up);

        static ARENA: &LinearAllocator = static_allocator!(1000);
        let mut map = BitGrid::new(ARENA, (5, 4)).unwrap();
        map.set(TilePosition::new(1, 1), true);
        map.set(TilePosition::new(1, 2), true);
        map.set(TilePosition::new(3, 1), true);
        map.set(TilePosition::new(3, 2), true);
        map.set(TilePosition::new(4, 1), true);

        let path = find_path_to(
            TilePosition::new(0, 1),
            TilePosition::new(4, 2),
            &map,
            ARENA,
        );
        assert!(path.is_some(), "should be able to find the way");
        assert_eq!(
            expected_path.len(),
            path.unwrap().len(),
            "did not find the shortest path"
        );
    }
}
