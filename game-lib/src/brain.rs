//! Character behavior controllers.

use core::cmp::Reverse;

use arrayvec::ArrayVec;
use bytemuck::Zeroable;
use engine::{
    allocators::LinearAllocator, collections::FixedVec, define_system, game_objects::Scene,
};
use tracing::{debug, trace};

use crate::{
    GameTicks, Sprite,
    game_object::{
        CharacterStatus, JobStationStatus, JobStationVariant, Personality, Resource,
        ResourceVariant, Stockpile, StockpileReliantTag, TilePosition,
    },
    grid::BitGrid,
    notifications::{NotificationId, NotificationSet},
    pathfinding::{Direction, Path, find_path_to, find_path_to_any},
};

pub const MAX_GOALS: usize = 8;

#[derive(Debug)]
pub struct HaulDescription {
    resource: ResourceVariant,
    amount: u8,
    destination: (JobStationVariant, TilePosition),
}

#[derive(Debug)]
pub enum Goal {
    Work {
        haul_wait_timeout: Option<(NotificationId, GameTicks)>,
        job: JobStationVariant,
    },
    Haul {
        description: HaulDescription,
    },
    FollowPath {
        from: TilePosition,
        path: Path,
    },
    Relax {
        relax_start_tick: GameTicks,
        walk_aabb: (TilePosition, TilePosition),
    },
    RefillOxygen,
    // TODO: Add a goal or another way to "stop" a character while an animation
    // or notification effect is happening (maybe an Animatable component or
    // something?)
}

impl Goal {
    pub fn sprite(&self, personality: Personality) -> Option<Sprite> {
        match self {
            Goal::Work { .. } => Some(Sprite::GoalWork),
            Goal::Haul { .. } => Some(Sprite::GoalHaul),
            Goal::FollowPath { .. } => None,
            Goal::Relax { .. } if personality.contains(Personality::KAOMOJI) => {
                Some(Sprite::GoalRelaxAlt)
            }
            Goal::Relax { .. } => Some(Sprite::GoalRelax),
            Goal::RefillOxygen => Some(Sprite::GoalOxygen),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Occupation {
    Idle,
    Hauler,
    Operator(JobStationVariant),
}

const OCCUPATION_LIST: [Occupation; 4] = [
    Occupation::Idle,
    Occupation::Hauler,
    Occupation::Operator(JobStationVariant::ENERGY_GENERATOR),
    Occupation::Operator(JobStationVariant::OXYGEN_GENERATOR),
];

impl Occupation {
    pub fn sprite(&self, _personality: Personality) -> Option<Sprite> {
        match self {
            Occupation::Idle => Some(Sprite::OccupationIdle),
            Occupation::Hauler => Some(Sprite::OccupationHauler),
            Occupation::Operator(JobStationVariant::ENERGY_GENERATOR) => {
                Some(Sprite::OccupationWorkEnergy)
            }
            Occupation::Operator(JobStationVariant::OXYGEN_GENERATOR) => {
                Some(Sprite::OccupationWorkOxygen)
            }
            Occupation::Operator(_) => None,
        }
    }

    pub fn previous(self) -> Occupation {
        if let Some(idx) = OCCUPATION_LIST.iter().position(|occ| *occ == self) {
            let len = OCCUPATION_LIST.len();
            OCCUPATION_LIST[(idx + len - 1) % len]
        } else {
            debug_assert!(false, "unrecognized occupation: {self:?}");
            Occupation::Idle
        }
    }

    pub fn next(self) -> Occupation {
        if let Some(idx) = OCCUPATION_LIST.iter().position(|occ| *occ == self) {
            OCCUPATION_LIST[(idx + 1) % OCCUPATION_LIST.len()]
        } else {
            debug_assert!(false, "unrecognized occupation: {self:?}");
            Occupation::Idle
        }
    }
}

#[derive(Debug)]
pub struct Brain {
    pub goal_stack: ArrayVec<Goal, MAX_GOALS>,
    pub job: Occupation,
    pub max_haul_amount: u8,
    pub wait_ticks: GameTicks,
    pub ticks_without_goal: GameTicks,
    pub has_relaxed: bool,
}

impl Brain {
    pub fn new() -> Brain {
        Brain {
            goal_stack: ArrayVec::new(),
            job: Occupation::Idle,
            max_haul_amount: 2,
            wait_ticks: 30,
            ticks_without_goal: 0,
            has_relaxed: false,
        }
    }

    pub fn next_move_direction(&self) -> Option<Direction> {
        if let Some(Goal::FollowPath { path, .. }) = self.goal_stack.last() {
            Some(path.into_iter().next()?)
        } else {
            None
        }
    }

    pub fn current_job(&self) -> Option<JobStationVariant> {
        if let Some(Goal::Work { job, .. }) = self.goal_stack.last() {
            Some(*job)
        } else {
            None
        }
    }

    pub fn update_goals(
        &mut self,
        (current_brain_index, current_position, current_tick): (u8, TilePosition, GameTicks),
        scene: &mut Scene,
        haul_notifications: &mut NotificationSet<HaulDescription>,
        walls: &BitGrid,
        temp_arena: &mut LinearAllocator,
    ) {
        let span = tracing::info_span!("", current_brain_index);
        let _enter = span.enter();

        let mut hashed_bytes = ArrayVec::<u8, 13>::new();
        for bytes in [
            &[current_brain_index][..],
            &current_position.x.to_le_bytes()[..],
            &current_position.y.to_le_bytes()[..],
            &current_tick.to_le_bytes()[..],
        ] {
            let result = hashed_bytes.try_extend_from_slice(bytes);
            debug_assert!(result.is_ok());
        }
        let rand = seahash::hash(&hashed_bytes);

        let mut current_status = CharacterStatus::zeroed();
        scene.run_system(define_system!(|_, characters: &[CharacterStatus]| {
            for character in characters {
                if character.brain_index == current_brain_index {
                    current_status = *character;
                    break;
                }
            }
        }));

        if current_status.oxygen == 0 {
            self.goal_stack.clear();
            // TODO: display/animate running out of oxygen
            return;
        }
        let demoralized = current_status.morale <= CharacterStatus::LOW_MORALE_THRESHOLD;

        // This branch picks something occupation-based to do, so it's not ran
        // when on low morale.
        if self.goal_stack.is_empty() && !demoralized {
            match self.job {
                Occupation::Idle => {
                    // Idling!
                    debug!("idling");
                }
                Occupation::Operator(job) => {
                    debug!("finding work at {job:?}");
                    self.goal_stack.push(Goal::Work {
                        haul_wait_timeout: None,
                        job,
                    });
                }
                Occupation::Hauler => {
                    // Find the closest haul job (by destination) and take it
                    trace!("finding hauling work to do");

                    let Some(mut hauls_by_distance) =
                        FixedVec::new(temp_arena, haul_notifications.len())
                    else {
                        debug_assert!(false, "not enough memory for haul distance calculation");
                        return;
                    };
                    for (id, desc) in haul_notifications.iter() {
                        let dist = desc.destination.1.manhattan_distance(*current_position);
                        let could_add = hauls_by_distance.push((id, dist));
                        debug_assert!(could_add.is_ok());
                    }
                    hauls_by_distance.sort_unstable_by_key(|(_, dist)| Reverse(*dist));

                    let capacity_left = temp_arena.total() - temp_arena.allocated();
                    let mut temp_arena = LinearAllocator::new(temp_arena, capacity_left).unwrap();
                    while let Some((notif_id, _)) = hauls_by_distance.pop() {
                        temp_arena.reset();
                        if let Some(description) = haul_notifications.get_mut(notif_id) {
                            // Check that the destination is reachable
                            let dst = description.destination;
                            let path_to_dest =
                                find_path_to(current_position, dst.1, true, walls, &temp_arena);
                            if path_to_dest.is_none() {
                                continue;
                            }

                            // Check that the resource is reachable
                            let Some(dsts) = find_non_reserved_resources(
                                scene,
                                description.resource,
                                &temp_arena,
                                walls,
                            ) else {
                                continue;
                            };
                            let path_to_resource =
                                find_path_to_any(current_position, &dsts, true, walls, &temp_arena);
                            if path_to_resource.is_none() {
                                continue;
                            }

                            // Accept the job
                            debug!("hauling {description:?}");
                            if description.amount > self.max_haul_amount {
                                description.amount -= self.max_haul_amount;
                                self.goal_stack.push(Goal::Haul {
                                    description: HaulDescription {
                                        resource: description.resource,
                                        amount: self.max_haul_amount,
                                        destination: dst,
                                    },
                                });
                            } else {
                                let description = haul_notifications.remove(notif_id).unwrap();
                                self.goal_stack.push(Goal::Haul { description });
                            }
                        }
                    }
                }
            }
        }

        temp_arena.reset();

        if current_status.oxygen <= CharacterStatus::LOW_OXYGEN_THRESHOLD
            && self
                .goal_stack
                .iter()
                .all(|goal| !matches!(goal, Goal::RefillOxygen))
        {
            if let Some(oxygen) =
                find_non_reserved_resources(scene, ResourceVariant::OXYGEN, temp_arena, walls)
            {
                let from = current_position;
                if let Some(path) = find_path_to_any(from, &oxygen, true, walls, temp_arena) {
                    debug!("found path to oxygen: {path:?}");
                    self.goal_stack.push(Goal::RefillOxygen);
                    self.goal_stack.push(Goal::FollowPath { from, path });
                } else {
                    debug!("the tanks are runnign out but there's no oxygen to refill with :(");
                }
            } else {
                debug_assert!(false, "ran out of memory to find oxygen?");
            }
        }

        temp_arena.reset();

        if self.goal_stack.is_empty() {
            if self.ticks_without_goal >= self.wait_ticks || demoralized {
                self.goal_stack.push(Goal::Relax {
                    relax_start_tick: current_tick,
                    walk_aabb: (
                        TilePosition::new(
                            current_position.x.saturating_sub(5),
                            current_position.y.saturating_sub(5),
                        ),
                        TilePosition::new(
                            (current_position.x.saturating_add(5)).min(walls.width() as i16 - 1),
                            (current_position.y.saturating_add(5)).min(walls.height() as i16 - 1),
                        ),
                    ),
                });
            } else {
                self.ticks_without_goal += 1;
            }
        }

        let mut new_instrumental_goal = None;
        let mut goal_not_acheivable = false;
        let mut goal_finished = false;

        let Some(current_goal) = self.goal_stack.last_mut() else {
            return;
        };
        match current_goal {
            Goal::Work {
                haul_wait_timeout,
                job,
            } => {
                if demoralized {
                    goal_not_acheivable = true;
                }
                match self.job {
                    Occupation::Operator(job_) if *job == job_ => {} // keep working
                    _ => goal_finished = true, // occupation changed, done here
                }

                // See if we're ready to work, request resources if needed
                // (the actual work is done in work ticks upstream)
                let mut within_working_distance = false;
                scene.run_system(define_system!(
                    |_,
                     job_stations: &mut [JobStationStatus],
                     stockpiles: &mut [Stockpile],
                     positions: &[TilePosition]| {
                        for ((job_station, stockpile), pos) in
                            job_stations.iter_mut().zip(stockpiles).zip(positions)
                        {
                            if job_station.variant == *job
                                && current_position.manhattan_distance(**pos) < 2
                            {
                                within_working_distance = true;
                                if let Some(details) = job_station.variant.details() {
                                    let resources =
                                        stockpile.get_resources_mut(details.resource_variant);
                                    let current_amount = resources.map(|a| *a).unwrap_or(0);
                                    if current_amount >= details.resource_amount {
                                        if haul_wait_timeout.is_some() {
                                            *haul_wait_timeout = None;
                                            debug!("got resources while waiting");
                                        }
                                    } else if haul_wait_timeout.is_none() {
                                        let description = HaulDescription {
                                            resource: details.resource_variant,
                                            destination: (job_station.variant, *pos),
                                            amount: details.resource_amount,
                                        };
                                        debug!("requesting {description:?}");
                                        match haul_notifications.notify(description) {
                                            Ok(haul_id) => {
                                                *haul_wait_timeout =
                                                    Some((haul_id, self.wait_ticks));
                                            }
                                            Err(_) => {
                                                debug_assert!(
                                                    false,
                                                    "haul notification queue is full!",
                                                )
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                ));

                // Do the haul yourself if it's been too long
                if let Some((haul_id, ticks_left)) = haul_wait_timeout.take() {
                    let haul_still_waiting = haul_notifications.check(haul_id);
                    if ticks_left == 0 {
                        if haul_still_waiting {
                            // Do it yourself:
                            let description = haul_notifications.remove(haul_id).unwrap();
                            debug!(
                                "tired of waiting, hauling {}x {:?} myself",
                                description.amount, description.resource,
                            );
                            new_instrumental_goal = Some(Goal::Haul { description });
                        } else {
                            debug!(
                                "someone picked up the haul job, continuing work on the next tick",
                            );
                        }
                    } else {
                        // Keep waiting:
                        *haul_wait_timeout = Some((haul_id, ticks_left - 1));
                    }
                }

                // Not near a job station, but the goal is working: get to work
                if !within_working_distance {
                    // Mark suitable job stations on the grid
                    let Some(mut destinations) = BitGrid::new(temp_arena, walls.size()) else {
                        debug_assert!(false, "out of memory for pathfinding to job station :(");
                        return;
                    };
                    scene.run_system(define_system!(
                        |_, positions: &[TilePosition], job_stations: &[JobStationStatus]| {
                            for (pos, job_station) in positions.iter().zip(job_stations) {
                                if job_station.variant == *job {
                                    destinations.set(*pos, true);
                                    trace!("found potential job station at: {pos:?}");
                                }
                            }
                        }
                    ));

                    // Find path
                    let from = current_position;
                    if let Some(path) =
                        find_path_to_any(from, &destinations, true, walls, temp_arena)
                    {
                        debug!("found path to work: {path:?}");
                        new_instrumental_goal = Some(Goal::FollowPath { from, path });
                    } else {
                        debug!("could not find path to work :(");
                        goal_not_acheivable = true;
                        // TODO: signal that work isn't available?
                    }
                }
            }

            Goal::Haul {
                description:
                    HaulDescription {
                        resource,
                        destination,
                        amount: requested_amount,
                    },
            } => {
                // Try to pick the resource from the current tile
                let mut picked_up_thus_far = 0;
                scene.run_system(define_system!(
                    |_, positions: &[TilePosition], stockpiles: &mut [Stockpile]| {
                        for (position, stockpile) in positions.iter().zip(stockpiles) {
                            if position.manhattan_distance(*current_position) < 2
                                && stockpile.has_non_reserved_resources(*resource)
                            {
                                let stockpile_amount =
                                    stockpile.get_resources_mut(*resource).unwrap();
                                let picked_up =
                                    (*requested_amount - picked_up_thus_far).min(*stockpile_amount);
                                *stockpile_amount -= picked_up;
                                picked_up_thus_far += picked_up;
                                debug!("picked up {picked_up}x {resource:?}");
                            }
                            if picked_up_thus_far >= *requested_amount {
                                break;
                            }
                        }
                    }
                ));

                // Check if we have enough items in our stockpile (or move the
                // stuff we just picked up into our stockpile if we picked up
                // some)
                let mut resources_acquired = false;
                let mut current_amount = 0;
                scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], stockpiles: &mut [Stockpile]| {
                        for (character, stockpile) in characters.iter().zip(stockpiles) {
                            if character.brain_index == current_brain_index {
                                current_amount = stockpile.get_resources(*resource).unwrap_or(0);
                                if picked_up_thus_far > 0 && current_amount < *requested_amount {
                                    let pocketed =
                                        picked_up_thus_far.min(*requested_amount - current_amount);
                                    debug!("adding {pocketed}x {resource:?} to my stockpile");
                                    stockpile.add_resource(*resource, pocketed).unwrap();
                                    stockpile.mark_reserved(*resource, true);
                                    picked_up_thus_far -= pocketed;
                                    current_amount += pocketed;
                                }
                                if current_amount >= *requested_amount {
                                    resources_acquired = true;
                                }
                                break;
                            }
                        }
                    }
                ));

                if picked_up_thus_far > 0 {
                    debug!(
                        "could not fit all the resources in the character's stockpile, dropping the rest ({picked_up_thus_far}x {resource:?}) at {current_position:?}"
                    );
                    let dropped_resources = Resource {
                        position: current_position,
                        stockpile: Stockpile::zeroed().with_resource(
                            *resource,
                            picked_up_thus_far,
                            false,
                        ),
                        stockpile_reliant: StockpileReliantTag {},
                    };
                    if scene.spawn(dropped_resources).is_err() {
                        debug!(
                            "tried to pick up resources and managed to overflow the character's pockets *and the floor*"
                        );
                        debug_assert!(false, "resource game object table is too small");
                    }
                }

                if !resources_acquired {
                    debug!("looking for (more) {resource:?}");

                    // Find path
                    let destinations =
                        find_non_reserved_resources(scene, *resource, temp_arena, walls);
                    let from = current_position;
                    if let Some(path) = destinations
                        .and_then(|dsts| find_path_to_any(from, &dsts, true, walls, temp_arena))
                    {
                        debug!("found path to resource: {path:?}");
                        new_instrumental_goal = Some(Goal::FollowPath { from, path });
                    } else if current_amount > 0 {
                        debug!(
                            "could not find more resource {resource:?}, but I have {current_amount}, bringing what I got"
                        );
                        resources_acquired = true;
                    } else {
                        debug!("could not find resources, giving up");
                        goal_not_acheivable = true;
                    }
                }

                // Either find a path to the destination or the resources,
                // depending on if we have the stuff
                let mut drop_off = false;
                if resources_acquired {
                    debug!("I have {current_amount}x {resource:?} and am bringing them back");
                    let (from, to) = (current_position, destination.1);
                    if let Some(path) = find_path_to(from, to, true, walls, temp_arena) {
                        if path.is_empty() {
                            drop_off = true;
                            goal_finished = true;
                        } else {
                            debug!("bringing the stuff via {path:?}");
                            new_instrumental_goal = Some(Goal::FollowPath { from, path });
                        }
                    } else {
                        debug!("could not find path from {from:?} to {to:?}");
                    }
                }

                // We have the stuff, and are at the destination, drop the resources off
                if drop_off {
                    debug!("dropping off haul at {current_position:?}");

                    let (dst_job, dst_pos) = *destination;

                    // Add the resources to the job station's stockpile (if they fit)
                    let mut dropped_off = 0;
                    scene.run_system(define_system!(
                        |_,
                         job_stations: &[JobStationStatus],
                         positions: &[TilePosition],
                         stockpiles: &mut [Stockpile]| {
                            for ((job_station, position), stockpile) in
                                job_stations.iter().zip(positions).zip(stockpiles)
                            {
                                if job_station.variant == dst_job && *position == dst_pos {
                                    debug_assert!(
                                        position.manhattan_distance(*current_position) < 2
                                    );
                                    let overflow = stockpile
                                        .add_resource(*resource, current_amount)
                                        .err()
                                        .unwrap_or(0);
                                    dropped_off += current_amount - overflow;
                                    goal_finished = true;
                                    break;
                                }
                            }
                        }
                    ));

                    // Remove the dropped off amount from the hauler's stockpile
                    // and mark it as non-reserved
                    let mut left_over = 0;
                    scene.run_system(define_system!(
                        |_, characters: &[CharacterStatus], stockpiles: &mut [Stockpile]| {
                            for (character, stockpile) in characters.iter().zip(stockpiles) {
                                if character.brain_index == current_brain_index {
                                    if let Some(hauled_res) = stockpile.get_resources_mut(*resource)
                                    {
                                        left_over = *hauled_res - dropped_off;
                                        *hauled_res -= dropped_off;
                                        *hauled_res -= left_over;
                                        stockpile.mark_reserved(*resource, false);
                                    }
                                    break;
                                }
                            }
                        }
                    ));

                    if left_over > 0 {
                        debug!(
                            "destination did not need all this, leaving the leftovers here ({left_over}x {resource:?}) at {current_position:?}"
                        );
                        let dropped_resources = Resource {
                            position: current_position,
                            stockpile: Stockpile::zeroed()
                                .with_resource(*resource, left_over, false),
                            stockpile_reliant: StockpileReliantTag {},
                        };
                        if scene.spawn(dropped_resources).is_err() {
                            debug!(
                                "the leftovers could not fit on the floor (they have been removed from reality)"
                            );
                            debug_assert!(false, "resource game object table is too small");
                        }
                    }

                    if !goal_finished {
                        goal_not_acheivable = true;
                    }
                }
            }

            Goal::FollowPath { from, path } => {
                if path.is_empty() {
                    debug!("reached destination {from:?}");
                    goal_finished = true;
                } else if current_position != *from {
                    let mut destination = *from;
                    let mut steps_progressed = None;
                    for (i, step) in path.into_iter().enumerate() {
                        destination = destination + step;
                        if destination == current_position {
                            steps_progressed = Some(i + 1);
                        }
                    }

                    if let Some(steps_progressed) = steps_progressed {
                        // Progressed on the path, truncate the start of the path.
                        let mut truncated_path = Path::default();
                        for step in path.into_iter().skip(steps_progressed) {
                            truncated_path.add_step(step);
                        }
                        *from = current_position;
                        *path = truncated_path;
                        trace!("moved {steps_progressed} steps");
                    } else if let Some(new_path) =
                        find_path_to(current_position, destination, true, walls, temp_arena)
                    {
                        // Strayed off the path, make a new one.
                        *from = current_position;
                        *path = new_path;
                        debug!("strayed off, made a new path: {path:?}");
                    } else {
                        // Strayed off the path, and can't find a new one, give up.
                        goal_not_acheivable = true;
                        debug!("can't find destination {destination:?}");
                    }
                }
            }

            Goal::Relax {
                relax_start_tick,
                walk_aabb,
            } => {
                debug!("relaxing!");
                self.has_relaxed = true;
                if *relax_start_tick != current_tick {
                    // Started relaxing earlier, and ended up back at this goal,
                    // so call it finished. If there's nothing useful to do (or
                    // morale is low), the next tick's goal will be relax again.
                    goal_finished = true;
                } else {
                    // Try to find a spot to walk to:
                    let x = (rand >> 32) % walk_aabb.0.x.abs_diff(walk_aabb.1.x) as u64;
                    let y = (rand >> 32) % walk_aabb.0.y.abs_diff(walk_aabb.1.y) as u64;
                    let dst = TilePosition::new(x as i16, y as i16);
                    let from = current_position;
                    if let Some(path) = find_path_to(from, dst, true, walls, temp_arena) {
                        new_instrumental_goal = Some(Goal::FollowPath { from, path });
                    }
                }
            }

            Goal::RefillOxygen => {
                let mut oxygen_found = false;
                scene.run_system(define_system!(
                    |_, positions: &[TilePosition], stockpiles: &mut [Stockpile]| {
                        for (position, stockpile) in positions.iter().zip(stockpiles) {
                            if position.manhattan_distance(*current_position) < 2
                                && stockpile.has_non_reserved_resources(ResourceVariant::OXYGEN)
                            {
                                let stockpile_amount = stockpile
                                    .get_resources_mut(ResourceVariant::OXYGEN)
                                    .unwrap();
                                if *stockpile_amount > 0 {
                                    *stockpile_amount -= 1;
                                    oxygen_found = true;
                                    debug!(
                                        "found oxygen, left {} in the stockpile",
                                        *stockpile_amount,
                                    );
                                    break;
                                }
                            }
                        }
                    }
                ));

                if !oxygen_found {
                    goal_not_acheivable = true;
                } else if current_status.oxygen + 1 >= CharacterStatus::MAX_OXYGEN {
                    goal_finished = true;
                }

                scene.run_system(define_system!(|_, characters: &mut [CharacterStatus]| {
                    for character in characters {
                        if character.brain_index == current_brain_index {
                            character.oxygen = character.oxygen.saturating_add(1);
                            debug!(
                                "breathed in oxygen, now at {}/{}",
                                character.oxygen,
                                CharacterStatus::MAX_OXYGEN,
                            );
                            break;
                        }
                    }
                }));
            }
        }

        temp_arena.reset();

        if goal_not_acheivable {
            debug!("giving up on {:?}", self.goal_stack.last());
            self.goal_stack.pop();
        } else if goal_finished {
            debug!("finished {:?}", self.goal_stack.last());
            self.goal_stack.pop();
        } else if let Some(new_instrumental_goal) = new_instrumental_goal {
            debug!(
                "doing {new_instrumental_goal:?} first to be able to do {:?}",
                self.goal_stack.last(),
            );
            if self.goal_stack.try_push(new_instrumental_goal).is_err() {
                self.goal_stack.clear(); // reconsider everything
            }
        }
    }
}

fn find_non_reserved_resources<'a>(
    scene: &mut Scene,
    resource: ResourceVariant,
    temp_arena: &'a LinearAllocator,
    walls: &BitGrid,
) -> Option<BitGrid<'a>> {
    let Some(mut destinations) = BitGrid::new(temp_arena, walls.size()) else {
        debug_assert!(false, "out of memory for pathfinding to resource :(");
        return None;
    };
    scene.run_system(define_system!(
        |_, positions: &[TilePosition], stockpiles: &[Stockpile]| {
            for (pos, stockpile) in positions.iter().zip(stockpiles) {
                if stockpile.has_non_reserved_resources(resource) {
                    destinations.set(*pos, true);
                    trace!("found potential resource at: {pos:?}");
                }
            }
        }
    ));
    Some(destinations)
}
