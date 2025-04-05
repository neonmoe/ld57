#![no_std]

mod camera;
mod tilemap;

use camera::Camera;
use engine::{Engine, allocators::LinearAllocator, renderer::DrawQueue};
use glam::Vec2;
use platform::{Instant, Platform};
use tilemap::Tilemap;

#[repr(u8)]
enum DrawLayer {
    Tilemap,
}

pub struct Game {
    tilemap: Tilemap<'static>,
    camera: Camera,
}

impl Game {
    pub fn new(arena: &'static LinearAllocator, engine: &Engine) -> Game {
        Game {
            tilemap: Tilemap::new(arena, &engine.resource_db),
            camera: Camera {
                position: Vec2::ZERO,
                size: Vec2::ZERO,
                output_size: Vec2::ZERO,
            },
        }
    }

    pub fn iterate(&mut self, engine: &mut Engine, platform: &dyn Platform, _timestamp: Instant) {
        let (draw_width, draw_height) = platform.draw_area();
        let aspect_ratio = draw_width / draw_height;
        self.camera.output_size = Vec2::new(draw_width, draw_height);
        self.camera.size = Vec2::new(aspect_ratio * 16., 16.);
        self.camera.position = self.camera.size / 2.;

        let draw_scale = platform.draw_scale_factor();
        let mut draw_queue = DrawQueue::new(&engine.frame_arena, 10_000, draw_scale).unwrap();

        self.tilemap.render(
            &mut draw_queue,
            &engine.resource_db,
            &mut engine.resource_loader,
            &self.camera,
            &engine.frame_arena,
        );

        draw_queue.dispatch_draw(&engine.frame_arena, platform);
    }
}
