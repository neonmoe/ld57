use core::{f32::consts::PI, fmt::Write};

use arrayvec::ArrayString;
use bytemuck::Zeroable;
use engine::{
    allocators::LinearAllocator,
    collections::FixedVec,
    geom::Rect,
    renderer::DrawQueue,
    resources::{ResourceDatabase, ResourceLoader, sprite::SpriteHandle},
};
use glam::{USizeVec2, Vec2};
use libm::{ceilf, cosf, floorf, sinf};

use crate::{DrawLayer, camera::Camera, grid::Grid};

#[derive(Clone, Copy, Debug, Zeroable)]
#[repr(u8)]
pub enum Tile {
    Seafloor,
    Wall,
    _Count,
}

pub struct Tilemap<'a> {
    tiles: Grid<'a, Tile>,
    tile_sprites: FixedVec<'a, SpriteHandle>,
}

impl Tilemap<'_> {
    pub fn new<'a>(arena: &'a LinearAllocator, resources: &ResourceDatabase) -> Tilemap<'a> {
        let (width, height) = (128, 128);
        let mut tiles = Grid::new_zeroed(arena, (width, height)).unwrap();
        for y in 0..height {
            for x in 0..width {
                let noise = perlin_noise(Vec2::new(x as f32, y as f32) / 4.0);
                tiles[(x, y)] = if noise > -0.2 {
                    Tile::Seafloor
                } else {
                    Tile::Wall
                };
            }
        }

        let tile_types: [Tile; Tile::_Count as usize] = [Tile::Seafloor, Tile::Wall];
        let mut tile_sprites = FixedVec::new(arena, Tile::_Count as usize).unwrap();
        for tile in tile_types {
            let mut name = ArrayString::<27>::new();
            write!(&mut name, "{tile:?}").expect("tile name too long");
            let sprite = resources.find_sprite(&name).unwrap();
            tile_sprites.push(sprite).unwrap();
        }

        Tilemap {
            tiles,
            tile_sprites,
        }
    }

    pub fn render(
        &self,
        draw_queue: &mut DrawQueue,
        resources: &ResourceDatabase,
        resource_loader: &mut ResourceLoader,
        camera: &Camera,
        temp_arena: &LinearAllocator,
    ) {
        let top_left = (camera.position - camera.size / 2.)
            .max(Vec2::ZERO)
            .as_usizevec2();
        let bottom_right = (camera.position + camera.size / 2.)
            .max(Vec2::ZERO)
            .ceil()
            .as_usizevec2()
            .min(USizeVec2::new(self.tiles.width(), self.tiles.height()));

        let mut tile_sprites = FixedVec::new(temp_arena, self.tile_sprites.len()).unwrap();
        for sprite in &*self.tile_sprites {
            let _ = tile_sprites.push(resources.get_sprite(*sprite));
        }

        let scale = camera.output_size / camera.size;
        for y in top_left.y..bottom_right.y {
            for x in top_left.x..bottom_right.x {
                let tile = self.tiles[(x, y)];
                let Some(sprite) = tile_sprites.get(tile as usize) else {
                    debug_assert!(false, "missing sprite for tile: {tile:?}");
                    continue;
                };
                let dst = Rect::xywh(x as f32 * scale.x, y as f32 * scale.y, scale.x, scale.y);
                let _ = sprite.draw(
                    dst,
                    DrawLayer::Tilemap as u8,
                    draw_queue,
                    resources,
                    resource_loader,
                );
            }
        }

        // TODO: draw an "outline" on tile edges between differing tiles
    }
}

fn perlin_noise(sample_point: Vec2) -> f32 {
    let corners = [
        sample_point.floor(),
        sample_point.ceil(),
        Vec2::new(floorf(sample_point.x), ceilf(sample_point.y)),
        Vec2::new(ceilf(sample_point.x), floorf(sample_point.y)),
    ];
    let mut values = [0.0; 4];
    for (i, corner) in corners.into_iter().enumerate() {
        let offset = sample_point - corner;
        let hash = seahash::hash(bytemuck::bytes_of(&corner.as_ivec2())) & 0xFF;
        let angle = (hash as f32 / 0xFF as f32) * 2. * PI;
        let gradient = Vec2::new(cosf(angle), sinf(angle));
        let value = gradient.dot(offset);
        values[i] = value;
    }
    let xt = (sample_point.x - floorf(sample_point.x)).abs();
    let yt = (sample_point.y - floorf(sample_point.y)).abs();
    let a = values[0] + smoothstep(xt) * (values[1] - values[0]);
    let b = values[2] + smoothstep(xt) * (values[3] - values[2]);
    a + smoothstep(yt) * (b - a)
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let x2 = x * x;
    3. * x2 - 2. * x2 * x
}
