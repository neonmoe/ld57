#![no_std]

mod brain;
mod camera;
mod game_object;
mod grid;
mod notifications;
mod pathfinding;
mod tilemap;

use core::time::Duration;

use brain::{Brain, HaulDescription, Occupation};
use bytemuck::Zeroable;
use camera::Camera;
use engine::{
    Engine,
    allocators::LinearAllocator,
    collections::FixedVec,
    define_system,
    game_objects::{GameObjectHandle, Scene},
    geom::Rect,
    renderer::DrawQueue,
    resources::sprite::SpriteHandle,
};
use game_object::{
    Character, CharacterStatus, JobStation, JobStationStatus, JobStationVariant, Resource,
    ResourceVariant, Stockpile, StockpileReliantTag, TilePosition,
};
use glam::Vec2;
use grid::BitGrid;
use notifications::NotificationSet;
use platform::{Instant, Platform};
use tilemap::Tilemap;
use tracing::debug;

const MAX_CHARACTERS: usize = 10;

pub type GameTicks = u64;
pub const MILLIS_PER_TICK: u64 = 10;

#[repr(u8)]
enum DrawLayer {
    Tilemap,
    GameObjects,
}

pub struct Game {
    tilemap: Tilemap<'static>,
    camera: Camera,
    scene: Scene<'static>,
    brains: FixedVec<'static, Brain>,
    haul_notifications: NotificationSet<'static, HaulDescription>,
    current_tick: u64,
    next_tick_time: Instant,
    placeholder_sprite: SpriteHandle,
}

impl Game {
    pub fn new(arena: &'static LinearAllocator, engine: &Engine, platform: &dyn Platform) -> Game {
        let mut brains = FixedVec::new(arena, MAX_CHARACTERS).unwrap();
        brains.push(Brain::new()).unwrap();
        brains[0].job = Occupation::Operator(JobStationVariant::ENERGY_GENERATOR);

        let haul_notifications = NotificationSet::new(arena, 128).unwrap();

        let mut scene = Scene::builder()
            .with_game_object_type::<Character>(MAX_CHARACTERS)
            .with_game_object_type::<JobStation>(100)
            .with_game_object_type::<Resource>(1000)
            .build(arena, &engine.frame_arena)
            .unwrap();

        let char_spawned = scene.spawn(Character {
            status: CharacterStatus { brain_index: 0 },
            position: TilePosition::new(5, 1),
            held: Stockpile::zeroed(),
        });
        debug_assert!(char_spawned.is_ok());

        let job_station_spawned = scene.spawn(JobStation {
            position: TilePosition::new(7, 7),
            stockpile: Stockpile::zeroed().with_resource(ResourceVariant::MAGMA, 0, true),
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
                    stockpile: Stockpile::zeroed().with_resource(ResourceVariant::MAGMA, 2, false),
                    stockpile_reliant: StockpileReliantTag {},
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
            brains,
            haul_notifications,
            current_tick: 0,
            next_tick_time: platform.now(),
            placeholder_sprite: engine.resource_db.find_sprite("Placeholder").unwrap(),
        }
    }

    pub fn iterate(&mut self, engine: &mut Engine, platform: &dyn Platform, timestamp: Instant) {
        // Game logic:

        while timestamp >= self.next_tick_time {
            self.next_tick_time = self.next_tick_time + Duration::from_millis(MILLIS_PER_TICK);
            self.current_tick += 1;

            // Each tick can reuse the entire frame arena, since it's such a top level thing
            engine.frame_arena.reset();

            // Reserve some of the frame arena for one-function-call-long allocations e.g. pathfinding
            let mut temp_arena = LinearAllocator::new(&engine.frame_arena, 1024 * 1024).unwrap();

            let walls = BitGrid::new(&engine.frame_arena, self.tilemap.tiles.size()).unwrap();
            // TODO: splat all the stuff in the world to walls

            if let Some(mut brains_to_think) = FixedVec::new(&engine.frame_arena, MAX_CHARACTERS) {
                self.scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], positions: &[TilePosition]| {
                        for (character, pos) in characters.iter().zip(positions) {
                            let _ = brains_to_think.push((character.brain_index, *pos));
                        }
                    }
                ));

                for (brain_idx, pos) in &mut *brains_to_think {
                    self.brains[*brain_idx].update_goals(
                        (*brain_idx, *pos),
                        &mut self.scene,
                        &mut self.haul_notifications,
                        &walls,
                        &temp_arena,
                    );
                    temp_arena.reset();
                }
            }

            let on_move_tick = self.current_tick % 30 == 0;
            let on_work_tick = self.current_tick % 30 == 10 || self.current_tick % 30 == 20;

            let mut workers = FixedVec::new(&engine.frame_arena, MAX_CHARACTERS).unwrap();
            self.scene.run_system(define_system!(
                |_, characters: &[CharacterStatus], positions: &[TilePosition]| {
                    for (character, pos) in characters.iter().zip(positions) {
                        if let Some(job) = self.brains[character.brain_index].current_job() {
                            let could_record_worker = workers.push((job, *pos));
                            debug_assert!(could_record_worker.is_ok());
                        }
                    }
                }
            ));

            // Move all characters who are currently following a path
            if on_move_tick {
                self.scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], positions: &mut [TilePosition]| {
                        for (character, pos) in characters.iter().zip(positions) {
                            let brain = &self.brains[character.brain_index];
                            if let Some(new_pos) = brain.next_move_position() {
                                *pos = new_pos;
                            }
                        }
                    }
                ));
            }

            // Produce at all job stations with a worker next to it
            if on_work_tick {
                self.scene.run_system(define_system!(
                    |_,
                     jobs: &mut [JobStationStatus],
                     stockpiles: &mut [Stockpile],
                     positions: &[TilePosition]| {
                        for ((job, stockpile), pos) in
                            jobs.iter_mut().zip(stockpiles).zip(positions)
                        {
                            for (worker_job, worker_position) in workers.iter() {
                                if job.variant == *worker_job && worker_position == pos {
                                    if let Some(details) = job.details() {
                                        let resources =
                                            stockpile.get_resources_mut(details.resource_variant);
                                        let current_amount =
                                            resources.as_ref().map(|a| **a).unwrap_or(0);
                                        if current_amount >= details.resource_amount {
                                            job.work_invested += 1;
                                            if job.work_invested >= details.work_amount {
                                                job.work_invested -= details.work_amount;
                                                if let Some(resources) = resources {
                                                    *resources -= details.resource_amount;
                                                }
                                                stockpile.insert_resource(
                                                    details.output_variant,
                                                    details.output_amount,
                                                );
                                                debug!(
                                                    "produced {}x {:?} at {pos:?}",
                                                    details.output_amount, details.output_variant
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                ));
            }

            // Clean up empty stockpiles
            if let Some(mut empty_piles) = FixedVec::<GameObjectHandle>::new(&temp_arena, 100) {
                self.scene.run_system(define_system!(
                    |handles, stockpiles: &[Stockpile], _tags: &[StockpileReliantTag]| {
                        for (handle, stockpile) in handles.zip(stockpiles) {
                            if stockpile.is_empty() {
                                let delete_result = empty_piles.push(handle);
                                if delete_result.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                ));
                let _ = self.scene.delete(&mut empty_piles);
            } else {
                debug_assert!(false, "not enough memory to collect the garbage stockpiles");
            }
            temp_arena.reset();
        }

        // Render:

        let (draw_width, draw_height) = platform.draw_area();
        let draw_scale = platform.draw_scale_factor();
        let aspect_ratio = draw_width / draw_height;
        self.camera.output_size = Vec2::new(draw_width, draw_height);
        self.camera.size = Vec2::new(aspect_ratio * 16., 16.);
        self.camera.position = self.camera.size / 2.;

        let mut draw_queue = DrawQueue::new(&engine.frame_arena, 10_000, draw_scale).unwrap();

        self.tilemap.render(
            &mut draw_queue,
            &engine.resource_db,
            &mut engine.resource_loader,
            &self.camera,
            &engine.frame_arena,
        );

        // Draw placeholders for every game object with a TilePosition
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
