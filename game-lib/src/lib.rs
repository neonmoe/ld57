#![no_std]

mod brain;
mod camera;
mod game_object;
mod grid;
mod notifications;
mod pathfinding;
mod tilemap;

use core::{fmt::Write, time::Duration};

use arrayvec::{ArrayString, ArrayVec};
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
    resources::{
        ResourceDatabase, ResourceLoader, audio_clip::AudioClipHandle, sprite::SpriteHandle,
    },
};
use game_object::{
    Character, CharacterStatus, Collider, JobStation, JobStationStatus, JobStationVariant,
    Resource, ResourceVariant, Stockpile, StockpileReliantTag, TilePosition,
};
use glam::Vec2;
use grid::BitGrid;
use notifications::NotificationSet;
use platform::{Instant, Platform};
use tilemap::{Tile, Tilemap};
use tracing::debug;

const MAX_CHARACTERS: usize = 10;

pub type GameTicks = u64;
pub const MILLIS_PER_TICK: u64 = 100;

#[derive(Clone, Copy)]
#[repr(u8)]
enum DrawLayer {
    Tilemap,
    LooseStockpiles,
    CharacterSuits,
    CharacterHelmets,
    JobStationStockpiles,
    CarriedStockpiles,
}

#[derive(Clone, Copy)]
#[repr(usize)]
enum AudioChannel {
    Music,
}

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
enum Sprite {
    Placeholder,
    Helmet,
    Suit,
    Energy,
    Magma,
    _Count,
}

pub struct Game {
    tilemap: Tilemap<'static>,
    camera: Camera,
    scene: Scene<'static>,
    brains: FixedVec<'static, Brain>,
    haul_notifications: NotificationSet<'static, HaulDescription>,
    current_tick: u64,
    next_tick_time: Instant,
    sprites: ArrayVec<SpriteHandle, { Sprite::_Count as usize }>,
    music_clips: ArrayVec<AudioClipHandle, 4>,
    last_music_clip_start: Instant,
}

impl Game {
    pub fn new(arena: &'static LinearAllocator, engine: &Engine, platform: &dyn Platform) -> Game {
        let mut brains = FixedVec::new(arena, MAX_CHARACTERS).unwrap();
        brains.push(Brain::new()).unwrap();
        brains.push(Brain::new()).unwrap();
        brains[0].job = Occupation::Operator(JobStationVariant::ENERGY_GENERATOR);
        brains[1].job = Occupation::Hauler;

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
            collider: Collider::NOT_WALKABLE,
        });
        debug_assert!(char_spawned.is_ok());

        let char_spawned = scene.spawn(Character {
            status: CharacterStatus { brain_index: 1 },
            position: TilePosition::new(6, 4),
            held: Stockpile::zeroed(),
            collider: Collider::NOT_WALKABLE,
        });
        debug_assert!(char_spawned.is_ok());

        let job_station_spawned = scene.spawn(JobStation {
            position: TilePosition::new(7, 7),
            stockpile: Stockpile::zeroed().with_resource(ResourceVariant::MAGMA, 0, true),
            status: JobStationStatus {
                variant: JobStationVariant::ENERGY_GENERATOR,
                work_invested: 0,
            },
            collider: Collider::NOT_WALKABLE,
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
            sprites: {
                use Sprite::*;
                let sprite_enums: [Sprite; Sprite::_Count as usize] =
                    [Placeholder, Helmet, Suit, Energy, Magma];
                let mut sprites = ArrayVec::new();
                for sprite in sprite_enums {
                    let mut name = ArrayString::<27>::new();
                    let _ = write!(&mut name, "{sprite:?}");
                    sprites.push(engine.resource_db.find_sprite(&name).unwrap());
                }
                sprites
            },
            music_clips: {
                let mut music_clips = ArrayVec::new();
                for i in 0..music_clips.capacity() {
                    let mut name = ArrayString::<27>::new();
                    let _ = write!(&mut name, "Soundtrack{i:02}");
                    if let Some(clip) = engine.resource_db.find_audio_clip(&name) {
                        music_clips.push(clip);
                    }
                }
                music_clips
            },
            last_music_clip_start: platform.now() - Duration::from_secs(10000),
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

            // Set up this tick's collision information
            let mut walls = BitGrid::new(&engine.frame_arena, self.tilemap.tiles.size()).unwrap();
            self.scene.run_system(define_system!(
                |_, colliders: &[Collider], positions: &[TilePosition]| {
                    for (collider, pos) in colliders.iter().zip(positions) {
                        if collider.is_not_walkable() {
                            walls.set(*pos, true);
                        }
                    }
                }
            ));
            for y in 0..self.tilemap.tiles.height() {
                for x in 0..self.tilemap.tiles.width() {
                    match self.tilemap.tiles[(x, y)] {
                        Tile::Wall => walls.set(TilePosition::new(x as i16, y as i16), true),
                        Tile::Seafloor => {}
                        Tile::_Count => debug_assert!(false, "Tile::_Count in the tilemap?"),
                    }
                }
            }

            // Run the think tick for the brains
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
                        &mut temp_arena,
                    );
                    temp_arena.reset();
                }
            }

            let on_move_tick = self.current_tick % 3 == 0;
            let on_work_tick = self.current_tick % 3 != 0;

            // Set up this tick's working worker information
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
                                if job.variant == *worker_job
                                    && worker_position.manhattan_distance(**pos) < 2
                                {
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

        // Music:
        if let Some(duration) = timestamp.duration_since(self.last_music_clip_start) {
            if duration > Duration::from_secs(45) {
                let time_ms = timestamp
                    .duration_since(Instant::reference())
                    .unwrap_or_else(|| Instant::reference().duration_since(timestamp).unwrap())
                    .as_micros();
                let hash = seahash::hash(&time_ms.to_le_bytes()) as usize;
                self.last_music_clip_start = timestamp;
                engine.audio_mixer.play_clip(
                    AudioChannel::Music as usize,
                    self.music_clips[hash % self.music_clips.len()],
                    false,
                    &engine.resource_db,
                );
            }
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

        let scale = self.camera.output_size / self.camera.size;

        // Non-specific stockpiles
        self.scene.run_system(define_system!(
            |_,
             tile_positions: &[TilePosition],
             stockpiles: &[Stockpile],
             _tags: &[StockpileReliantTag]| {
                for (tile_pos, stockpile) in tile_positions.iter().zip(stockpiles) {
                    draw_stockpile(
                        &engine.resource_db,
                        &mut engine.resource_loader,
                        &mut draw_queue,
                        DrawLayer::LooseStockpiles,
                        &self.sprites,
                        scale,
                        tile_pos,
                        stockpile,
                    );
                }
            }
        ));

        // Characters' stockpiles
        self.scene.run_system(define_system!(
            |_,
             tile_positions: &[TilePosition],
             stockpiles: &[Stockpile],
             _chars: &[CharacterStatus]| {
                for (tile_pos, stockpile) in tile_positions.iter().zip(stockpiles) {
                    draw_stockpile(
                        &engine.resource_db,
                        &mut engine.resource_loader,
                        &mut draw_queue,
                        DrawLayer::CarriedStockpiles,
                        &self.sprites,
                        scale,
                        tile_pos,
                        stockpile,
                    );
                }
            }
        ));

        // Job stations' stockpiles
        self.scene.run_system(define_system!(
            |_,
             tile_positions: &[TilePosition],
             stockpiles: &[Stockpile],
             _job_stations: &[JobStationStatus]| {
                for (tile_pos, stockpile) in tile_positions.iter().zip(stockpiles) {
                    draw_stockpile(
                        &engine.resource_db,
                        &mut engine.resource_loader,
                        &mut draw_queue,
                        DrawLayer::JobStationStockpiles,
                        &self.sprites,
                        scale,
                        tile_pos,
                        stockpile,
                    );
                }
            }
        ));

        // Characters
        let helmet_sprite = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::Helmet as usize]);
        let suit_sprite = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::Suit as usize]);
        self.scene.run_system(define_system!(
            |_, tile_positions: &[TilePosition], _characters: &[CharacterStatus]| {
                for tile_pos in tile_positions {
                    let helmet = (
                        DrawLayer::CharacterHelmets,
                        helmet_sprite,
                        Rect::xywh(
                            (tile_pos.x as f32 + 0.5 / 2.) * scale.x,
                            (tile_pos.y as f32 - 0.2 / 3.) * scale.y,
                            scale.x * 1. / 2.,
                            scale.y * 1. / 2.,
                        ),
                    );
                    let suit = (
                        DrawLayer::CharacterSuits,
                        suit_sprite,
                        Rect::xywh(
                            (tile_pos.x as f32 + 0. / 3.) * scale.x,
                            (tile_pos.y as f32 + 0. / 3.) * scale.y,
                            scale.x * 3. / 3.,
                            scale.y * 3. / 3.,
                        ),
                    );
                    for (layer, sprite, dst) in [helmet, suit] {
                        let draw_success = sprite.draw(
                            dst,
                            layer as u8,
                            &mut draw_queue,
                            &engine.resource_db,
                            &mut engine.resource_loader,
                        );
                        debug_assert!(draw_success);
                    }
                }
            }
        ));

        draw_queue.dispatch_draw(&engine.frame_arena, platform);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stockpile(
    resources: &ResourceDatabase,
    resource_loader: &mut ResourceLoader,
    draw_queue: &mut DrawQueue,
    layer: DrawLayer,
    sprites: &[SpriteHandle],
    scale: Vec2,
    tile_pos: &TilePosition,
    stockpile: &Stockpile,
) {
    for i in 0..stockpile.variant_count as usize {
        let stockpile_pos = [
            Vec2::new(0.3, 0.75),
            Vec2::new(0.6, 0.5),
            Vec2::new(0.2, 0.25),
        ][i];
        for j in 0..stockpile.amounts[i].min(5) as usize {
            let individual_offset = [
                Vec2::new(-0.1, -0.07),
                Vec2::new(0.1, 0.02),
                Vec2::new(0.0, -0.08),
                Vec2::new(-0.05, 0.02),
                Vec2::new(0.05, -0.03),
            ][j];
            let off = stockpile_pos + individual_offset;
            let dst = Rect::xywh(
                (tile_pos.x as f32 + off.x) * scale.x,
                (tile_pos.y as f32 + off.y) * scale.y,
                scale.x / 4.,
                scale.y / 4.,
            );
            let sprite = stockpile.variants[i]
                .sprite()
                .unwrap_or(Sprite::Placeholder);
            let sprite = resources.get_sprite(sprites[sprite as usize]);
            let draw_success =
                sprite.draw(dst, layer as u8, draw_queue, resources, resource_loader);
            debug_assert!(draw_success);
        }
    }
}
