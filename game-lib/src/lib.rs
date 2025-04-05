#![no_std]

mod camera;
mod game_object;
mod tilemap;

use bytemuck::Zeroable;
use camera::Camera;
use engine::{
    Engine, allocators::LinearAllocator, define_system, game_objects::Scene, geom::Rect,
    renderer::DrawQueue, resources::sprite::SpriteHandle,
};
use game_object::{
    Character, CharacterStatus, JobStation, JobStationStatus, JobStationVariant, Resource,
    ResourceVariant, Stockpile, TilePosition,
};
use glam::Vec2;
use platform::{Instant, Platform};
use tilemap::Tilemap;

#[repr(u8)]
enum DrawLayer {
    Tilemap,
    GameObjects,
}

pub struct Game {
    tilemap: Tilemap<'static>,
    camera: Camera,
    scene: Scene<'static>,
    placeholder_sprite: SpriteHandle,
}

impl Game {
    pub fn new(arena: &'static LinearAllocator, engine: &Engine) -> Game {
        let mut scene = Scene::builder()
            .with_game_object_type::<Character>(10)
            .with_game_object_type::<JobStation>(100)
            .with_game_object_type::<Resource>(1000)
            .build(arena, &engine.frame_arena)
            .unwrap();

        let char_spawned = scene.spawn(Character {
            status: CharacterStatus { brain_index: 0 }, // TODO: implement brains
            position: TilePosition::new(5, 1),
            held: Stockpile::zeroed(),
        });
        debug_assert!(char_spawned.is_ok());

        let job_station_spawned = scene.spawn(JobStation {
            position: TilePosition::new(7, 7),
            stockpile: Stockpile::zeroed(),
            status: JobStationStatus {
                variant: JobStationVariant::ENERGY_GENERATOR,
                work_invested: 0,
            },
        });
        debug_assert!(job_station_spawned.is_ok());

        for y in 4..7 {
            for x in 1..4 {
                let res_spawned = scene.spawn(Resource {
                    position: TilePosition::new(x, y),
                    stockpile: Stockpile::zeroed().with_resource(ResourceVariant::MAGMA, 2),
                });
                debug_assert!(res_spawned.is_ok());
            }
        }

        Game {
            tilemap: Tilemap::new(arena, &engine.resource_db),
            camera: Camera {
                position: Vec2::ZERO,
                size: Vec2::ZERO,
                output_size: Vec2::ZERO,
            },
            scene,
            placeholder_sprite: engine.resource_db.find_sprite("Placeholder").unwrap(),
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

        let placeholder_sprite = engine.resource_db.get_sprite(self.placeholder_sprite);
        let scale = self.camera.output_size / self.camera.size;
        self.scene
            .run_system(define_system!(|_, tile_positions: &[TilePosition]| {
                for tile_pos in tile_positions {
                    let dst = Rect::xywh(
                        tile_pos.x as f32 * scale.x,
                        tile_pos.y as f32 * scale.y,
                        scale.x,
                        scale.y,
                    );
                    let draw_success = placeholder_sprite.draw(
                        dst,
                        DrawLayer::GameObjects as u8,
                        &mut draw_queue,
                        &engine.resource_db,
                        &mut engine.resource_loader,
                    );
                    debug_assert!(draw_success);
                }
            }));

        draw_queue.dispatch_draw(&engine.frame_arena, platform);
    }
}
