//! Character behavior controllers.

use arrayvec::ArrayVec;
use engine::{collections::FixedVec, define_system, game_objects::Scene};

use crate::game_object::{
    JobStation, JobStationStatus, JobStationVariant, Stockpile, TilePosition,
};

#[derive(Debug)]
enum Goal {
    Work,
    Haul,
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
            job: JobStationVariant::NONE,
        }
    }

    pub fn update_goals(&mut self, current_position: TilePosition, scene: &mut Scene) {
        if self.goal_stack.is_empty() {
            self.goal_stack.push(Goal::Work);
        }

        match self.goal_stack.last().unwrap() {
            Goal::Work => {
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
                                    if current_amount < details.resource_amount {
                                        // TODO: needs more resources
                                    }
                                }
                                break;
                            }
                        }
                    }
                ));

                if !within_working_distance {
                    // TODO: find work
                }
            }
            Goal::Haul => {}
        }
    }
}
