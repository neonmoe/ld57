//! Character behavior controllers.

use core::time::Duration;

use arrayvec::ArrayVec;
use bytemuck::Zeroable;
use engine::{define_system, game_objects::Scene};
use platform::Instant;

use crate::{
    game_object::{
        CharacterStatus, JobStationStatus, JobStationVariant, ResourceVariant, Stockpile,
        TilePosition,
    },
    notifications::{NotificationId, NotificationSet},
};

#[derive(Debug)]
pub struct HaulDescription {
    resource: ResourceVariant,
    destination: (JobStationVariant, TilePosition),
}

#[derive(Debug)]
enum Goal {
    Work {
        waiting_on_haul_id: Option<(NotificationId, Instant)>,
    },
    Haul {
        description: HaulDescription,
    },
}

#[derive(Debug)]
pub struct Brain {
    goal_stack: ArrayVec<Goal, 8>,
    pub job: JobStationVariant,
}

impl Brain {
    pub fn new() -> Brain {
        Brain {
            goal_stack: ArrayVec::new(),
            job: JobStationVariant::zeroed(),
        }
    }

    pub fn update_goals(
        &mut self,
        current_brain_index: usize,
        current_position: TilePosition,
        time: Instant,
        scene: &mut Scene,
        haul_notifications: &mut NotificationSet<HaulDescription>,
    ) {
        if self.goal_stack.is_empty() {
            self.goal_stack.push(Goal::Work {
                waiting_on_haul_id: None,
            });
        }

        match self.goal_stack.last_mut().unwrap() {
            Goal::Work { waiting_on_haul_id } => {
                let mut within_working_distance = false;

                scene.run_system(define_system!(
                    |_,
                     jobs: &mut [JobStationStatus],
                     stockpiles: &mut [Stockpile],
                     positions: &[TilePosition]| {
                        for ((job, stockpile), pos) in
                            jobs.iter_mut().zip(stockpiles).zip(positions)
                        {
                            if job.variant == self.job
                                && current_position.manhattan_distance(**pos) < 2
                            {
                                within_working_distance = true;
                                if let Some(details) = job.details() {
                                    let resources =
                                        stockpile.get_resources_mut(details.resource_variant);
                                    let current_amount = resources.map(|a| *a).unwrap_or(0);
                                    if current_amount < details.resource_amount
                                        && waiting_on_haul_id.is_none()
                                    {
                                        let description = HaulDescription {
                                            resource: details.resource_variant,
                                            destination: (job.variant, *pos),
                                        };
                                        match haul_notifications.notify(description) {
                                            Ok(haul_id) => {
                                                *waiting_on_haul_id = Some((haul_id, time));
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

                if let Some((haul_id, req_time)) = waiting_on_haul_id.take() {
                    let haul_still_waiting = haul_notifications.check(haul_id);
                    if haul_still_waiting
                        && time.duration_since(req_time).unwrap_or(Duration::ZERO)
                            > Duration::from_secs(3)
                    {
                        // Do it yourself:
                        let description = haul_notifications.remove(haul_id).unwrap();
                        self.goal_stack.push(Goal::Haul { description });
                    } else {
                        // Keep waiting:
                        *waiting_on_haul_id = Some((haul_id, time));
                    }
                }

                if !within_working_distance {
                    // TODO: find work
                }
            }
            Goal::Haul {
                description:
                    HaulDescription {
                        resource,
                        destination,
                    },
            } => {
                let mut resources_acquired = false;
                scene.run_system(define_system!(
                    |_, characters: &[CharacterStatus], stockpiles: &[Stockpile]| {
                        for (character, stockpile) in characters.iter().zip(stockpiles) {
                            if character.brain_index == current_brain_index {
                                let current_amount =
                                    stockpile.get_resources(*resource).unwrap_or(0);
                                if current_amount > 0 {
                                    resources_acquired = true;
                                }
                                break;
                            }
                        }
                    }
                ));

                if resources_acquired {
                    // TODO: haul stuff to the destination
                } else {
                    // TODO: find resources to haul
                }
            }
        }
    }
}
