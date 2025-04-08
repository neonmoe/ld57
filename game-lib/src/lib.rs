#![no_std]

mod brain;
mod camera;
mod game_object;
mod grid;
mod menu;
mod notifications;
mod pathfinding;
mod tilemap;

use core::{fmt::Write, time::Duration};

use arrayvec::{ArrayString, ArrayVec};
use brain::{Brain, HaulDescription};
use bytemuck::Zeroable;
use camera::Camera;
use engine::{
    Engine,
    allocators::LinearAllocator,
    collections::FixedVec,
    define_system,
    game_objects::{GameObjectHandle, Scene},
    geom::Rect,
    input::{ActionKind, ActionState, InputDeviceState},
    renderer::DrawQueue,
    resources::{
        ResourceDatabase, ResourceLoader,
        audio_clip::AudioClipHandle,
        sprite::{SpriteAsset, SpriteHandle},
    },
};
use game_object::{
    Character, CharacterStatus, Collider, JobStation, JobStationStatus, JobStationVariant,
    Personality, Resource, ResourceVariant, Stockpile, StockpileReliantTag, TilePosition,
};
use glam::Vec2;
use grid::BitGrid;
use menu::{Menu, MenuAction, MenuEntry, MenuMode};
use notifications::NotificationSet;
use pathfinding::Direction;
use platform::{ActionCategory, Event, InputDevice, Instant, Platform};
use tilemap::{Tile, Tilemap};
use tracing::debug;

const MAX_CHARACTERS: usize = 10;

pub type GameTicks = u64;
pub const MILLIS_PER_TICK: u64 = 100;
const STOCKPILE_VISUALIZED_COUNT: u8 = 5;

#[derive(Clone, Copy)]
#[repr(u8)]
enum DrawLayer {
    // The map
    Tilemap,
    // Game objects
    LooseStockpiles,
    _ReserveFiveLooseStockpiles = DrawLayer::LooseStockpiles as u8 + STOCKPILE_VISUALIZED_COUNT,
    CharacterSuits,
    CharacterHelmets,
    JobStations,
    JobStationStockpiles,
    _ReserveFiveJobStationStockpiles =
        DrawLayer::JobStationStockpiles as u8 + STOCKPILE_VISUALIZED_COUNT,
    CarriedStockpiles,
    _ReserveFiveCarriedStockpiles = DrawLayer::CarriedStockpiles as u8 + STOCKPILE_VISUALIZED_COUNT,
    // UI
    Passes,
    PassInformation,
    PassGoalPile,
    MenuBg,
    MenuItems,
    MenuFg,
    _ReserveThreeSetsOfMenus = DrawLayer::MenuFg as u8 + 6,
    ControlsInfo,
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
    Pass,
    PassSelectionOverlay,
    GoalRelax,
    GoalRelaxAlt,
    GoalHaul,
    GoalWork,
    GoalOxygen,
    OccupationIdle,
    OccupationHauler,
    OccupationWorkEnergy,
    OccupationWorkOxygen,
    MenuBgTop,
    MenuBgMid,
    MenuBgBot,
    MenuUnderscore,
    MenuItemContinue,
    MenuItemQuit,
    MenuItemOptions,
    MenuItemManageChars,
    MenuItemBuild,
    MenuItemVolume,
    MenuItemFlipACfalse,
    MenuItemFlipACtrue,
    EnergyGenerator,
    OxygenGenerator,
    Oxygen,
    SliderHandle,
    Controls,
    ControlsFlipConfirm,
    _Count,
}

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
enum Button {
    Up = 0,
    Down,
    Left,
    Right,
    OpenMenu,
    Accept,
    Cancel,
    _Count,
}

fn create_action_bindings(
    device: InputDevice,
    flip_confirm_cancel: bool,
    platform: &dyn Platform,
) -> InputDeviceState<{ Button::_Count as usize }> {
    InputDeviceState {
        device,
        actions: [
            // Up
            ActionState {
                kind: ActionKind::Instant,
                mapping: platform.default_button_for_action(ActionCategory::Up, device),
                disabled: false,
                pressed: false,
            },
            // Down
            ActionState {
                kind: ActionKind::Instant,
                mapping: platform.default_button_for_action(ActionCategory::Down, device),
                disabled: false,
                pressed: false,
            },
            // Left
            ActionState {
                kind: ActionKind::Instant,
                mapping: platform.default_button_for_action(ActionCategory::Left, device),
                disabled: false,
                pressed: false,
            },
            // Right
            ActionState {
                kind: ActionKind::Instant,
                mapping: platform.default_button_for_action(ActionCategory::Right, device),
                disabled: false,
                pressed: false,
            },
            // OpenMenu
            ActionState {
                kind: ActionKind::Instant,
                mapping: platform.default_button_for_action(ActionCategory::Pause, device),
                disabled: false,
                pressed: false,
            },
            // Accept
            ActionState {
                kind: ActionKind::Instant,
                mapping: if flip_confirm_cancel {
                    platform.default_button_for_action(ActionCategory::Cancel, device)
                } else {
                    platform.default_button_for_action(ActionCategory::Accept, device)
                },
                disabled: false,
                pressed: false,
            },
            // Cancel
            ActionState {
                kind: ActionKind::Instant,
                mapping: if flip_confirm_cancel {
                    platform.default_button_for_action(ActionCategory::Accept, device)
                } else {
                    platform.default_button_for_action(ActionCategory::Cancel, device)
                },
                disabled: false,
                pressed: false,
            },
        ],
    }
}

pub struct Game {
    tilemap: Tilemap<'static>,
    camera: Camera,
    ui_camera: Camera,
    scene: Scene<'static>,
    brains: FixedVec<'static, Brain>,
    haul_notifications: NotificationSet<'static, HaulDescription>,
    current_tick: u64,
    next_tick_time: Instant,
    last_frame_timestamp: Instant,
    sprites: ArrayVec<SpriteHandle, { Sprite::_Count as usize }>,
    number_sprites: ArrayVec<SpriteHandle, 5>,
    music_clips: ArrayVec<AudioClipHandle, 4>,
    last_music_clip_start: Instant,
    flip_confirm_cancel: bool,
    input: Option<InputDeviceState<{ Button::_Count as usize }>>,
    paused: bool,
    menu: Option<MenuMode>,
}

impl Game {
    pub fn new(
        arena: &'static LinearAllocator,
        engine: &Engine,
        platform: &dyn Platform,
        seed: u64,
    ) -> Game {
        let mut brains = FixedVec::new(arena, MAX_CHARACTERS).unwrap();
        brains.push(Brain::new()).unwrap();
        brains.push(Brain::new()).unwrap();
        brains.push(Brain::new()).unwrap();
        brains.push(Brain::new()).unwrap();
        brains[0].wait_ticks = 50;
        brains[1].wait_ticks = 20;
        brains[2].wait_ticks = 40;
        brains[3].wait_ticks = 30;

        let characters = [
            CharacterStatus {
                brain_index: 0,
                oxygen: CharacterStatus::MAX_OXYGEN,
                morale: CharacterStatus::MAX_MORALE - 3,
                oxygen_depletion_amount: CharacterStatus::BASE_OXYGEN_DEPLETION_AMOUNT,
                morale_depletion_amount: CharacterStatus::BASE_MORALE_DEPLETION_AMOUNT,
                morale_relaxing_increment: CharacterStatus::BASE_MORALE_RELAXING_INCREMENT,
                personality: Personality::zeroed(),
            },
            CharacterStatus {
                brain_index: 1,
                oxygen: CharacterStatus::MAX_OXYGEN - 3,
                morale: CharacterStatus::MAX_MORALE,
                oxygen_depletion_amount: CharacterStatus::BASE_OXYGEN_DEPLETION_AMOUNT,
                morale_depletion_amount: CharacterStatus::BASE_MORALE_DEPLETION_AMOUNT + 2,
                morale_relaxing_increment: CharacterStatus::BASE_MORALE_RELAXING_INCREMENT + 2,
                personality: Personality::KAOMOJI,
            },
            CharacterStatus {
                brain_index: 2,
                oxygen: CharacterStatus::MAX_OXYGEN - 1,
                morale: CharacterStatus::MAX_MORALE - 2,
                oxygen_depletion_amount: CharacterStatus::BASE_OXYGEN_DEPLETION_AMOUNT,
                morale_depletion_amount: CharacterStatus::BASE_MORALE_DEPLETION_AMOUNT - 1,
                morale_relaxing_increment: CharacterStatus::BASE_MORALE_RELAXING_INCREMENT - 1,
                personality: Personality::zeroed(),
            },
            CharacterStatus {
                brain_index: 3,
                oxygen: CharacterStatus::MAX_OXYGEN - 2,
                morale: CharacterStatus::MAX_MORALE - 1,
                oxygen_depletion_amount: CharacterStatus::BASE_OXYGEN_DEPLETION_AMOUNT + 1,
                morale_depletion_amount: CharacterStatus::BASE_MORALE_DEPLETION_AMOUNT,
                morale_relaxing_increment: CharacterStatus::BASE_MORALE_RELAXING_INCREMENT + 2,
                personality: Personality::KAOMOJI,
            },
        ];

        let haul_notifications = NotificationSet::new(arena, 128).unwrap();

        let mut scene = Scene::builder()
            .with_game_object_type::<Character>(MAX_CHARACTERS)
            .with_game_object_type::<JobStation>(100)
            .with_game_object_type::<Resource>(2000)
            .build(arena, &engine.frame_arena)
            .unwrap();

        let mut tilemap = Tilemap::new(arena, &engine.resource_db, seed);
        let start_pos = 'pick_start_pos: {
            let (w, h) = tilemap.tiles.size();
            for y in h / 2 - 8..h / 2 + 8 {
                for x in w / 2 - 8..w / 2 + 8 {
                    if matches!(tilemap.tiles[(x, y)], Tile::Seafloor) {
                        break 'pick_start_pos TilePosition::new(x as i16, y as i16);
                    }
                }
            }
            TilePosition::new(64, 64)
        };

        // Spawn characters around start position
        for (i, character) in characters.into_iter().enumerate() {
            let x = start_pos.x - 1 + i as i16;
            let y = start_pos.y - 1 + ((i as i16 * 3 + 3) % 5);
            let position = TilePosition::new(x, y);
            let char_spawned = scene.spawn(Character {
                status: character,
                position,
                held: Stockpile::zeroed(),
                collider: Collider::NOT_WALKABLE,
            });
            debug_assert!(char_spawned.is_ok());
        }

        // Clear a start area
        for y in start_pos.y - 4..start_pos.y + 4 {
            for x in start_pos.x - 4..start_pos.x + 4 {
                let pos = TilePosition::new(x, y);
                tilemap.tiles[pos] = Tile::Seafloor;
            }
        }
        tilemap.tiles[start_pos + Direction::Left + Direction::Up + Direction::Up] =
            Tile::GeothermalVent;

        // Place the machines (TODO: remove after building is possible)
        let job_station_spawned = scene.spawn(JobStation {
            position: TilePosition::new(start_pos.x - 4, start_pos.y + 2),
            stockpile: Stockpile::zeroed().with_resource(ResourceVariant::MAGMA, 0, true),
            status: JobStationStatus {
                variant: JobStationVariant::ENERGY_GENERATOR,
                work_invested: 0,
            },
            collider: Collider::NOT_WALKABLE,
        });
        debug_assert!(job_station_spawned.is_ok());

        let job_station_spawned = scene.spawn(JobStation {
            position: TilePosition::new(start_pos.x, start_pos.y - 4),
            stockpile: Stockpile::zeroed().with_resource(ResourceVariant::ENERGY, 0, true),
            status: JobStationStatus {
                variant: JobStationVariant::OXYGEN_GENERATOR,
                work_invested: 0,
            },
            collider: Collider::NOT_WALKABLE,
        });
        debug_assert!(job_station_spawned.is_ok());

        // Spawn magma resources
        for y in 0..tilemap.tiles.height() as i16 {
            for x in 0..tilemap.tiles.width() as i16 {
                let position = TilePosition::new(x, y);
                if matches!(tilemap.tiles[position], Tile::GeothermalVent) {
                    for dir in Direction::ALL {
                        let position = position + dir;
                        if tilemap.tiles.in_bounds(position)
                            && matches!(tilemap.tiles[position], Tile::Seafloor)
                        {
                            let res_spawned = scene.spawn(Resource {
                                position,
                                stockpile: Stockpile::zeroed().with_resource(
                                    ResourceVariant::MAGMA,
                                    2,
                                    false,
                                ),
                                stockpile_reliant: StockpileReliantTag {},
                            });
                            debug_assert!(res_spawned.is_ok());
                        }
                    }
                }
            }
        }

        let mut main_menu = ArrayVec::new();
        main_menu.push(Menu::main_menu());

        Game {
            tilemap,
            camera: Camera {
                position: Vec2::new(start_pos.x as f32, start_pos.y as f32),
                size: Vec2::ZERO,
                output_size: Vec2::ZERO,
            },
            ui_camera: Camera {
                position: Vec2::ZERO,
                size: Vec2::ZERO,
                output_size: Vec2::ZERO,
            },
            scene,
            brains,
            haul_notifications,
            current_tick: 0,
            next_tick_time: platform.now(),
            last_frame_timestamp: platform.now(),
            sprites: {
                use Sprite::*;
                let sprite_enums: [Sprite; Sprite::_Count as usize] = [
                    Placeholder,
                    Helmet,
                    Suit,
                    Energy,
                    Magma,
                    Pass,
                    PassSelectionOverlay,
                    GoalRelax,
                    GoalRelaxAlt,
                    GoalHaul,
                    GoalWork,
                    GoalOxygen,
                    OccupationIdle,
                    OccupationHauler,
                    OccupationWorkEnergy,
                    OccupationWorkOxygen,
                    MenuBgTop,
                    MenuBgMid,
                    MenuBgBot,
                    MenuUnderscore,
                    MenuItemContinue,
                    MenuItemQuit,
                    MenuItemOptions,
                    MenuItemManageChars,
                    MenuItemBuild,
                    MenuItemVolume,
                    MenuItemFlipACfalse,
                    MenuItemFlipACtrue,
                    EnergyGenerator,
                    OxygenGenerator,
                    Oxygen,
                    SliderHandle,
                    Controls,
                    ControlsFlipConfirm,
                ];
                let mut sprites = ArrayVec::new();
                for sprite in sprite_enums {
                    let mut name = ArrayString::<27>::new();
                    let _ = write!(&mut name, "{sprite:?}");
                    sprites.push(engine.resource_db.find_sprite(&name).unwrap());
                }
                sprites
            },
            number_sprites: {
                let mut sprites = ArrayVec::new();
                for n in 1..=5 {
                    let mut name = ArrayString::<27>::new();
                    let _ = write!(&mut name, "Number{n}");
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
            flip_confirm_cancel: false,
            input: None,
            paused: true,
            menu: Some(MenuMode::MenuStack(main_menu)),
        }
    }

    pub fn iterate(&mut self, engine: &mut Engine, platform: &dyn Platform, timestamp: Instant) {
        let dt_real = timestamp
            .duration_since(self.last_frame_timestamp)
            .map(|d| d.as_secs_f32())
            .unwrap_or(0.0);
        self.last_frame_timestamp = timestamp;

        // Handle input:

        if let Some(event) = engine.event_queue.last() {
            match event.event {
                Event::DigitalInputPressed(device, _) | Event::DigitalInputReleased(device, _) => {
                    self.input = Some(create_action_bindings(
                        device,
                        self.flip_confirm_cancel,
                        platform,
                    ));
                }
            }
        }

        if let Some(input) = &mut self.input {
            input.update(&mut engine.event_queue);

            if input.actions[Button::OpenMenu as usize].pressed && !self.paused {
                self.paused = true;
                let mut menus = ArrayVec::new();
                menus.push(Menu::main_menu());
                self.menu = Some(MenuMode::MenuStack(menus));
            }

            if input.actions[Button::Cancel as usize].pressed {
                if let Some(MenuMode::MenuStack(menus)) = &mut self.menu {
                    menus.pop();
                    if menus.is_empty() {
                        self.menu = None;
                        self.paused = false;
                    }
                }
            }

            if let Some(menus) = self.menu.as_mut().and_then(|menus| {
                if let MenuMode::MenuStack(menus) = menus {
                    Some(menus)
                } else {
                    None
                }
            }) {
                if let Some(top_menu) = menus.last_mut() {
                    if let Some(selected) = top_menu.update(input) {
                        match selected {
                            (MenuEntry::Quit, MenuAction::Select) => platform.exit(true),
                            (MenuEntry::Continue, MenuAction::Select) => {
                                self.paused = false;
                                self.menu = None;
                            }
                            (MenuEntry::Options, MenuAction::Select) => {
                                menus.push(Menu::options(self.flip_confirm_cancel));
                            }
                            (MenuEntry::Build, MenuAction::Select) => {} // TODO
                            (MenuEntry::BuildSelect(_), MenuAction::Select) => {} // TODO
                            (MenuEntry::ManageCharacters, MenuAction::Select) => {
                                menus.push(Menu::manage_characters(self.brains.len()));
                            }
                            (MenuEntry::ManageCharacter { brain_index }, MenuAction::Previous) => {
                                let job = &mut self.brains[*brain_index].job;
                                *job = job.previous();
                            }
                            (MenuEntry::ManageCharacter { brain_index }, MenuAction::Next) => {
                                let job = &mut self.brains[*brain_index].job;
                                *job = job.next();
                            }
                            (MenuEntry::FlipAcceptCancel(flip), _) => {
                                *flip = !*flip;
                                self.flip_confirm_cancel = *flip;
                            }
                            (MenuEntry::Volume, MenuAction::Next) => {
                                let vol = &mut engine.audio_mixer.channels[0].volume;
                                *vol = vol.saturating_add(32);
                            }
                            (MenuEntry::Volume, MenuAction::Previous) => {
                                let vol = &mut engine.audio_mixer.channels[0].volume;
                                *vol = vol.saturating_sub(32);
                            }
                            _ => {}
                        }
                    }
                }
            }

            if !self.paused {
                let dx = (input.actions[Button::Right as usize].pressed as i32 as f32)
                    - (input.actions[Button::Left as usize].pressed as i32 as f32);
                let dy = (input.actions[Button::Down as usize].pressed as i32 as f32)
                    - (input.actions[Button::Up as usize].pressed as i32 as f32);
                self.camera.position += Vec2::new(dx, dy);
            }
        }

        // Game logic:

        while timestamp >= self.next_tick_time {
            self.next_tick_time = self.next_tick_time + Duration::from_millis(MILLIS_PER_TICK);
            if self.paused {
                continue;
            }
            self.current_tick += 1;

            let on_move_tick = self.current_tick % 3 == 0;
            let on_work_tick = self.current_tick % 2 == 0;
            let on_oxygen_and_morale_tick = self.current_tick % 100 == 0;
            let on_magma_spawn_tick = self.current_tick % 120 == 0;

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
                        Tile::Wall | Tile::GeothermalVent => {
                            walls.set(TilePosition::new(x as i16, y as i16), true)
                        }
                        Tile::Seafloor => {}
                        Tile::_Count => debug_assert!(false, "Tile::_Count in the tilemap?"),
                    }
                }
            }

            // Move all characters who are currently following a path
            // (specifically before the think tick, and updating the walls, so
            // that other characters can reroute based on the new position).
            if on_move_tick {
                self.scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], positions: &mut [TilePosition]| {
                        'next_char: for (character, pos) in characters.iter().zip(positions) {
                            let brain = &mut self.brains[character.brain_index as usize];
                            if let Some(dir) = brain.next_move_direction() {
                                let mut new_pos = *pos + dir;
                                let mut backup_dir = dir.next_clockwise();
                                while walls.get(new_pos) {
                                    new_pos = *pos + backup_dir;
                                    backup_dir = backup_dir.next_clockwise();
                                    if backup_dir == dir {
                                        // Walls in all directions, can't do much about that.
                                        continue 'next_char;
                                    }
                                }
                                walls.set(*pos, false);
                                walls.set(new_pos, true);
                                *pos = new_pos;
                            }
                        }
                    }
                ));
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
                    self.brains[*brain_idx as usize].update_goals(
                        (*brain_idx, *pos, self.current_tick),
                        &mut self.scene,
                        &mut self.haul_notifications,
                        &walls,
                        &mut temp_arena,
                    );
                    temp_arena.reset();
                }
            }

            // Set up this tick's working worker information
            let mut workers = FixedVec::new(&engine.frame_arena, MAX_CHARACTERS).unwrap();
            self.scene.run_system(define_system!(
                |_, characters: &[CharacterStatus], positions: &[TilePosition]| {
                    for (character, pos) in characters.iter().zip(positions) {
                        if let Some(job) = self.brains[character.brain_index as usize].current_job()
                        {
                            let could_record_worker = workers.push((job, *pos));
                            debug_assert!(could_record_worker.is_ok());
                        }
                    }
                }
            ));

            // Update oxygen and morale for all characters
            if on_oxygen_and_morale_tick {
                self.scene
                    .run_system(define_system!(|_, characters: &mut [CharacterStatus]| {
                        for character in characters {
                            let brain = &mut self.brains[character.brain_index as usize];
                            character.oxygen = (character.oxygen)
                                .saturating_sub(character.oxygen_depletion_amount);
                            if brain.has_relaxed {
                                character.morale = (character.morale)
                                    .saturating_add(character.morale_relaxing_increment)
                                    .min(CharacterStatus::MAX_MORALE);
                                brain.has_relaxed = false;
                            } else {
                                character.morale = (character.morale)
                                    .saturating_sub(character.morale_depletion_amount);
                            }
                        }
                    }));
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
                                    if let Some(details) = job.variant.details() {
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

            // Spawn magma
            if on_magma_spawn_tick {
                self.scene.run_system(define_system!(
                    |_, stockpiles: &mut [Stockpile], positions: &[TilePosition]| {
                        for (pos, stockpile) in positions.iter().zip(stockpiles) {
                            for dir in Direction::ALL {
                                let pos = *pos + dir;
                                if self.tilemap.tiles.in_bounds(pos)
                                    && matches!(self.tilemap.tiles[pos], Tile::GeothermalVent)
                                {
                                    let _ = stockpile.add_resource(ResourceVariant::MAGMA, 2);
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
                            if stockpile.is_empty()
                            // Don't despawn renewable magma
                                && stockpile.get_resources(ResourceVariant::MAGMA).is_none()
                            {
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
            if duration > Duration::from_secs(45) && !self.paused {
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
        self.ui_camera.output_size = Vec2::new(draw_width, draw_height);
        self.ui_camera.size = Vec2::new(aspect_ratio * 16., 16.);

        let mut draw_queue = DrawQueue::new(&engine.frame_arena, 10_000, draw_scale).unwrap();

        self.tilemap.render(
            &mut draw_queue,
            &engine.resource_db,
            &mut engine.resource_loader,
            &self.camera,
            &engine.frame_arena,
        );

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
                        &self.camera,
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
                        &self.camera,
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
                        &self.camera,
                        tile_pos,
                        stockpile,
                    );
                }
            }
        ));

        // Job stations themselves
        self.scene.run_system(define_system!(
            |_, tile_positions: &[TilePosition], job_stations: &[JobStationStatus]| {
                for (tile_pos, job_station) in tile_positions.iter().zip(job_stations) {
                    let sprite = job_station.variant.sprite();
                    let sprite = engine.resource_db.get_sprite(self.sprites[sprite as usize]);
                    let draw_success = sprite.draw(
                        self.camera.to_output(Rect::xywh(
                            tile_pos.x as f32,
                            tile_pos.y as f32,
                            1.,
                            1.,
                        )),
                        DrawLayer::JobStations as u8,
                        &mut draw_queue,
                        &engine.resource_db,
                        &mut engine.resource_loader,
                    );
                    debug_assert!(draw_success);
                }
            }
        ));

        // Characters on the map
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
                        self.camera.to_output(Rect::xywh(
                            tile_pos.x as f32 + 0.5 / 2.,
                            tile_pos.y as f32 - 0.2 / 3.,
                            1. / 2.,
                            1. / 2.,
                        )),
                    );
                    let suit = (
                        DrawLayer::CharacterSuits,
                        suit_sprite,
                        self.camera.to_output(Rect::xywh(
                            tile_pos.x as f32 + 0. / 3.,
                            tile_pos.y as f32 + 0. / 3.,
                            1.,
                            1.,
                        )),
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

        // Character passes for status
        let pass_sprite = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::Pass as usize]);
        let pass_selection_overlay_sprite = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::PassSelectionOverlay as usize]);
        self.scene
            .run_system(define_system!(|_, characters: &[CharacterStatus]| {
                for (i, character) in characters.iter().enumerate() {
                    let brain = &self.brains[character.brain_index as usize];

                    let selected_in_management = 'is_managed: {
                        if let Some(MenuMode::MenuStack(menus)) = &self.menu {
                            if let Some(top_menu) = menus.last() {
                                if let MenuEntry::ManageCharacter { brain_index } =
                                    top_menu.hover_entry()
                                {
                                    if brain_index == character.brain_index as usize {
                                        break 'is_managed true;
                                    }
                                }
                            }
                        }
                        false
                    };

                    const MAX_DRAWS: usize = 2 // The pass background and overlay
                        + 1 // Picture (and accessories, 0 currently)
                        + 1 // Occupation field
                        + brain::MAX_GOALS
                        + CharacterStatus::MAX_MORALE.div_ceil(5) as usize
                        + CharacterStatus::MAX_OXYGEN.div_ceil(5) as usize;
                    let mut draws = ArrayVec::<_, MAX_DRAWS>::new();

                    let mut pass_x = self.ui_camera.size.x / 2. - 5.7;
                    let pass_y = -self.ui_camera.size.y / 2. + 0.2 + i as f32 * 3.7;

                    if selected_in_management {
                        pass_x -= 1.0;
                    }

                    draws.push((
                        DrawLayer::Passes,
                        pass_sprite,
                        self.ui_camera
                            .to_output(Rect::xywh(pass_x, pass_y, 5.5, 3.5)),
                    ));

                    if selected_in_management {
                        draws.push((
                            DrawLayer::PassInformation,
                            pass_selection_overlay_sprite,
                            self.ui_camera
                                .to_output(Rect::xywh(pass_x, pass_y, 5.5, 3.5)),
                        ));
                    }

                    draws.extend(draw_counter(
                        &self.ui_camera,
                        &engine.resource_db,
                        &self.number_sprites,
                        character.morale,
                        pass_x + 2.4,
                        pass_y + 0.68,
                    ));

                    draws.extend(draw_counter(
                        &self.ui_camera,
                        &engine.resource_db,
                        &self.number_sprites,
                        character.oxygen,
                        pass_x + 2.4,
                        pass_y + 1.18,
                    ));

                    for (i, goal) in brain.goal_stack.iter().enumerate() {
                        if let Some(sprite) = goal.sprite(character.personality) {
                            let sprite =
                                engine.resource_db.get_sprite(self.sprites[sprite as usize]);
                            draws.push((
                                DrawLayer::PassGoalPile,
                                sprite,
                                self.ui_camera.to_output(Rect::xywh(
                                    pass_x + 0.2 + 0.2 * i as f32,
                                    pass_y + 1.65 + 0.1 * i as f32,
                                    3.3 / 2.,
                                    1.6 / 2.,
                                )),
                            ));
                        }
                    }

                    if let Some(sprite) = brain.job.sprite(character.personality) {
                        let sprite = engine.resource_db.get_sprite(self.sprites[sprite as usize]);
                        draws.push((
                            DrawLayer::PassInformation,
                            sprite,
                            self.ui_camera.to_output(Rect::xywh(
                                pass_x + 2.3,
                                pass_y + 0.22,
                                2.8,
                                0.4,
                            )),
                        ));
                    }

                    draws.push((
                        DrawLayer::PassInformation,
                        helmet_sprite,
                        self.ui_camera.to_output(Rect::xywh(
                            pass_x + 0.28,
                            pass_y + 0.31,
                            1.28,
                            1.28,
                        )),
                    ));

                    for (layer, sprite, dst) in draws {
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
            }));

        // Menus
        let menu_background_top = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::MenuBgTop as usize]);
        let menu_background_mid = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::MenuBgMid as usize]);
        let menu_background_bot = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::MenuBgBot as usize]);
        let menu_underscore = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::MenuUnderscore as usize]);
        let slider_handle = engine
            .resource_db
            .get_sprite(self.sprites[Sprite::SliderHandle as usize]);
        match &self.menu {
            Some(MenuMode::MenuStack(menus)) => {
                let last_menu_idx = menus.len().saturating_sub(1);
                for (rendered_idx, (menu_idx, menu)) in menus
                    .iter()
                    .enumerate()
                    .filter(|(_, menu)| menu.rendered)
                    .enumerate()
                {
                    let menu_camera = Camera {
                        position: self.ui_camera.size / 2.
                            - Vec2::new(0.2, 0.2)
                            - Vec2::new(2.0, 2.0) * (rendered_idx as f32),
                        size: self.ui_camera.size,
                        output_size: self.ui_camera.output_size,
                    };
                    let draw_layer_offset = rendered_idx as u8 * 3;

                    for (i, bg) in [menu_background_top]
                        .into_iter()
                        .chain(
                            [menu_background_mid]
                                .into_iter()
                                .cycle()
                                .take(menu.len())
                                .chain([menu_background_bot]),
                        )
                        .enumerate()
                    {
                        let draw_success = bg.draw(
                            menu_camera.to_output(Rect::xywh(0.0, i as f32, 5.5, 1.0)),
                            DrawLayer::MenuBg as u8 + draw_layer_offset,
                            &mut draw_queue,
                            &engine.resource_db,
                            &mut engine.resource_loader,
                        );
                        debug_assert!(draw_success);

                        let entry_idx = if i > 0 && i - 1 < menu.len() {
                            i - 1
                        } else {
                            continue;
                        };

                        if let Some(sprite) = menu.sprite(entry_idx) {
                            let sprite =
                                engine.resource_db.get_sprite(self.sprites[sprite as usize]);
                            let draw_success = sprite.draw(
                                menu_camera.to_output(Rect::xywh(0.25, i as f32 + 0.2, 5.0, 0.6)),
                                DrawLayer::MenuItems as u8 + draw_layer_offset,
                                &mut draw_queue,
                                &engine.resource_db,
                                &mut engine.resource_loader,
                            );
                            debug_assert!(draw_success);
                        }

                        if let MenuEntry::Volume = *menu.entry(entry_idx) {
                            let vol = engine.audio_mixer.channels[0].volume as f32 / 0xFF as f32;
                            let x = 0.25 + 2.0 + 2.6 * vol;
                            let draw_success = slider_handle.draw(
                                menu_camera.to_output(Rect::xywh(x, i as f32 + 0.3, 0.4, 0.4)),
                                DrawLayer::MenuFg as u8 + draw_layer_offset,
                                &mut draw_queue,
                                &engine.resource_db,
                                &mut engine.resource_loader,
                            );
                            debug_assert!(draw_success);
                        }

                        if entry_idx == menu.hover_index() && menu_idx == last_menu_idx {
                            let draw_success = menu_underscore.draw(
                                menu_camera.to_output(Rect::xywh(0.25, i as f32 + 0.8, 5.0, 0.1)),
                                DrawLayer::MenuFg as u8 + draw_layer_offset,
                                &mut draw_queue,
                                &engine.resource_db,
                                &mut engine.resource_loader,
                            );
                            debug_assert!(draw_success);
                        }
                    }
                }
            }
            Some(MenuMode::BuildPlacement) => todo!("build placement rendering"),
            None => {}
        }

        let controls_info_sprite = engine.resource_db.get_sprite(
            self.sprites[if self.flip_confirm_cancel {
                Sprite::ControlsFlipConfirm
            } else {
                Sprite::Controls
            } as usize],
        );
        {
            let y = self.ui_camera.size.y / 2. - 2.2;
            let draw_success = controls_info_sprite.draw(
                self.ui_camera
                    .to_output(Rect::xywh(-11. / 2., y, 11.0, 2.0)),
                DrawLayer::ControlsInfo as u8,
                &mut draw_queue,
                &engine.resource_db,
                &mut engine.resource_loader,
            );
            debug_assert!(draw_success);
        }

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
    camera: &Camera,
    tile_pos: &TilePosition,
    stockpile: &Stockpile,
) {
    for i in 0..stockpile.variant_count as usize {
        let stockpile_pos = [
            Vec2::new(0.3, 0.75),
            Vec2::new(0.6, 0.5),
            Vec2::new(0.2, 0.25),
        ][i];
        for j in 0..stockpile.amounts[i].min(STOCKPILE_VISUALIZED_COUNT) as usize {
            let individual_offset = [
                Vec2::new(-0.1, -0.07),
                Vec2::new(0.1, 0.02),
                Vec2::new(0.0, -0.08),
                Vec2::new(-0.05, 0.02),
                Vec2::new(0.05, -0.03),
            ][j];
            let off = stockpile_pos + individual_offset;
            let dst = camera.to_output(Rect::xywh(
                tile_pos.x as f32 + off.x,
                tile_pos.y as f32 + off.y,
                1. / 4.,
                1. / 4.,
            ));
            let sprite = stockpile.variants[i]
                .sprite()
                .unwrap_or(Sprite::Placeholder);
            let sprite = resources.get_sprite(sprites[sprite as usize]);
            let draw_success = sprite.draw(
                dst,
                layer as u8 + j as u8,
                draw_queue,
                resources,
                resource_loader,
            );
            debug_assert!(draw_success);
        }
    }
}

fn draw_counter<'a>(
    ui_camera: &Camera,
    resources: &'a ResourceDatabase,
    number_sprites: &[SpriteHandle],
    count: u8,
    x: f32,
    y: f32,
) -> impl Iterator<Item = (DrawLayer, &'a SpriteAsset, Rect)> {
    (0..count.div_ceil(5)).map(move |oxygen_i| {
        let count = (count - oxygen_i * 5).min(5) - 1;
        let number_sprite = resources.get_sprite(number_sprites[count as usize]);
        (
            DrawLayer::PassInformation,
            number_sprite,
            ui_camera.to_output(Rect::xywh(x + 0.4 * oxygen_i as f32, y, 0.4, 0.3)),
        )
    })
}
