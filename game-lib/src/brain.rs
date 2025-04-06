//! Character behavior controllers.

use arrayvec::ArrayVec;
use bytemuck::Zeroable;
use engine::{allocators::LinearAllocator, define_system, game_objects::Scene};
use tracing::{debug, trace};

use crate::{
    GameTicks,
    game_object::{
        CharacterStatus, JobStationStatus, JobStationVariant, Resource, ResourceVariant, Stockpile,
        StockpileReliantTag, TilePosition,
    },
    grid::BitGrid,
    notifications::{NotificationId, NotificationSet},
    pathfinding::{Path, find_path_to, find_path_to_any},
};

#[derive(Debug)]
pub struct HaulDescription {
    resource: ResourceVariant,
    amount: u8,
    destination: (JobStationVariant, TilePosition),
}

#[derive(Debug)]
enum Goal {
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
    // TODO: Add a goal or another way to "stop" a character while an animation
    // or notification effect is happening (maybe an Animatable component or
    // something?)
}

#[derive(Debug)]
pub enum Occupation {
    Idle,
    Hauler,
    Operator(JobStationVariant),
}

#[derive(Debug)]
pub struct Brain {
    goal_stack: ArrayVec<Goal, 8>,
    pub job: Occupation,
    pub max_haul_amount: u8,
    pub wait_ticks: GameTicks,
}

impl Brain {
    pub fn new() -> Brain {
        Brain {
            goal_stack: ArrayVec::new(),
            job: Occupation::Idle,
            max_haul_amount: 2,
            wait_ticks: 300,
        }
    }

    pub fn next_move_position(&self) -> Option<TilePosition> {
        if let Some(Goal::FollowPath { from, path }) = self.goal_stack.last() {
            Some(*from + path.into_iter().next()?)
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
        (current_brain_index, current_position): (usize, TilePosition),
        scene: &mut Scene,
        haul_notifications: &mut NotificationSet<HaulDescription>,
        walls: &BitGrid,
        temp_arena: &LinearAllocator,
    ) {
        let span = tracing::info_span!("", current_brain_index);
        let _enter = span.enter();

        if self.goal_stack.is_empty() {
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
                    let mut closest_haul = None;
                    let mut closest_haul_distance = u16::MAX;
                    for (id, desc) in haul_notifications.iter() {
                        let dist = desc.destination.1.manhattan_distance(*current_position);
                        if dist < closest_haul_distance {
                            closest_haul_distance = dist;
                            closest_haul = Some(id);
                        }
                    }
                    if let Some(notif_id) = closest_haul {
                        if let Some(description) = haul_notifications.get_mut(notif_id) {
                            debug!("hauling {description:?}");
                            if description.amount > self.max_haul_amount {
                                description.amount -= self.max_haul_amount;
                                self.goal_stack.push(Goal::Haul {
                                    description: HaulDescription {
                                        resource: description.resource,
                                        amount: self.max_haul_amount,
                                        destination: description.destination,
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

        if self.goal_stack.is_empty() {
            // Nothing useful to do
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
                                if let Some(details) = job_station.details() {
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
                    if haul_still_waiting && ticks_left == 0 {
                        // Do it yourself:
                        let description = haul_notifications.remove(haul_id).unwrap();
                        debug!(
                            "tired of waiting, hauling {}x {:?} myself",
                            description.amount, description.resource,
                        );
                        new_instrumental_goal = Some(Goal::Haul { description });
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
                    if let Some(path) = find_path_to_any(from, &destinations, walls, temp_arena) {
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
                            if *position == current_position
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
                scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], stockpiles: &mut [Stockpile]| {
                        for (character, stockpile) in characters.iter().zip(stockpiles) {
                            if character.brain_index == current_brain_index {
                                let mut current_amount =
                                    stockpile.get_resources(*resource).unwrap_or(0);
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

                // Either find a path to the destination or the resources,
                // depending on if we have the stuff
                let mut drop_off = false;
                if resources_acquired {
                    debug!("I have the needed {requested_amount}x {resource:?}");
                    let (from, to) = (current_position, destination.1);
                    if let Some(mut path) = find_path_to(from, to, walls, temp_arena) {
                        path.pop_step();
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
                } else {
                    debug!("looking for {resource:?}");

                    // Mark suitable resources on the grid
                    let Some(mut destinations) = BitGrid::new(temp_arena, walls.size()) else {
                        debug_assert!(false, "out of memory for pathfinding to resource :(");
                        return;
                    };
                    scene.run_system(define_system!(
                        |_, positions: &[TilePosition], stockpiles: &[Stockpile]| {
                            for (pos, stockpile) in positions.iter().zip(stockpiles) {
                                if stockpile.has_non_reserved_resources(*resource) {
                                    destinations.set(*pos, true);
                                    trace!("found potential resource at: {pos:?}");
                                }
                            }
                        }
                    ));

                    // Find path
                    let from = current_position;
                    if let Some(path) = find_path_to_any(from, &destinations, walls, temp_arena) {
                        debug!("found path to resource: {path:?}");
                        new_instrumental_goal = Some(Goal::FollowPath { from, path });
                    } else {
                        debug!(
                            "could not find resource {resource:?}, posting the notification again :("
                        );
                        goal_not_acheivable = true;
                        // Repost the haul notification for someone else to pick
                        // (the original poster won't know about this one, but
                        // that's fine).
                        let _ = haul_notifications.notify(HaulDescription {
                            resource: *resource,
                            destination: *destination,
                            amount: *requested_amount,
                        });
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
                                    debug_assert_eq!(
                                        1,
                                        position.manhattan_distance(*current_position),
                                    );
                                    let overflow = stockpile
                                        .add_resource(*resource, *requested_amount)
                                        .err()
                                        .unwrap_or(0);
                                    dropped_off += *requested_amount - overflow;
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
                                    let hauled_res =
                                        stockpile.get_resources_mut(*resource).unwrap();
                                    left_over = *hauled_res - dropped_off;
                                    *hauled_res -= dropped_off;
                                    *hauled_res -= left_over;
                                    stockpile.mark_reserved(*resource, false);
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
                        find_path_to(current_position, destination, walls, temp_arena)
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
        }

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
            self.goal_stack.push(new_instrumental_goal);
        }
    }
}
