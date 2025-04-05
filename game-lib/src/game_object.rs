use core::ops::Deref;

use bytemuck::{Pod, Zeroable};
use engine::impl_game_object;
use glam::IVec2;

// Game objects

#[derive(Debug, Zeroable)]
pub struct Character {
    pub status: CharacterStatus,
    pub position: TilePosition,
    pub held: Stockpile,
}
impl_game_object! {
    impl GameObject for Character using components {
        status: CharacterStatus,
        position: TilePosition,
        held: Stockpile,
    }
}

#[derive(Debug, Zeroable)]
pub struct Resource {
    pub position: TilePosition,
    pub stockpile: Stockpile,
}
impl_game_object! {
    impl GameObject for Resource using components {
        position: TilePosition,
        stockpile: Stockpile,
    }
}

#[derive(Debug, Zeroable)]
pub struct JobStation {
    pub position: TilePosition,
    pub stockpile: Stockpile,
    pub status: JobStationStatus,
}
impl_game_object! {
    impl GameObject for JobStation using components {
        position: TilePosition,
        stockpile: Stockpile,
        status: JobStationStatus,
    }
}

// Components

#[derive(Clone, Copy, Debug, Zeroable, Pod)]
#[repr(C)]
pub struct CharacterStatus {
    pub brain_index: usize,
}

#[derive(Clone, Copy, Debug, Zeroable, Pod)]
#[repr(C)]
pub struct JobStationStatus {
    pub variant: JobStationVariant,
    pub work_invested: u8,
}
impl JobStationStatus {
    pub const fn details(self) -> Option<JobStationDetails> {
        match self.variant {
            JobStationVariant::ENERGY_GENERATOR => Some(JobStationDetails {
                resource_variant: ResourceVariant::MAGMA,
                resource_amount: 1,
                work_amount: 10,
                output_variant: ResourceVariant::ENERGY,
                output_amount: 1,
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Zeroable, Pod)]
#[repr(C)]
pub struct Stockpile {
    pub variant_count: u8,
    pub variants: [ResourceVariant; 4],
    pub amounts: [u8; 4],
}
impl Stockpile {
    pub const fn with_resource(mut self, resource: ResourceVariant, amount: u8) -> Stockpile {
        let i = self.variant_count as usize;
        if i >= self.variants.len() {
            self
        } else {
            self.variants[i] = resource;
            self.amounts[i] = amount;
            self.variant_count += 1;
            self
        }
    }

    pub fn get_resources_mut(&mut self, variant: ResourceVariant) -> Option<&mut u8> {
        let len = self.variant_count as usize;
        for (variant_, amount) in self.variants[..len].iter().zip(&mut self.amounts[..len]) {
            if variant == *variant_ {
                return Some(amount);
            }
        }
        None
    }

    pub fn insert_resource(&mut self, variant: ResourceVariant, amount: u8) {
        let len = self.variant_count as usize;
        for (variant_, amount_) in self.variants[..len].iter().zip(&mut self.amounts[..len]) {
            if variant == *variant_ {
                *amount_ += amount;
                return;
            }
        }
        if len < self.variants.len() {
            self.variants[len] = variant;
            self.amounts[len] = amount;
            self.variant_count += 1;
        }
    }
}

#[derive(Clone, Copy, Debug, Zeroable, Pod)]
#[repr(C)]
pub struct TilePosition(pub IVec2);
impl TilePosition {
    pub fn new(x: i32, y: i32) -> TilePosition {
        TilePosition(IVec2 { x, y })
    }
}
impl Deref for TilePosition {
    type Target = IVec2;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// Other

pub struct JobStationDetails {
    pub resource_variant: ResourceVariant,
    pub resource_amount: u8,
    pub work_amount: u8,
    pub output_variant: ResourceVariant,
    pub output_amount: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Zeroable, Pod)]
#[repr(C)]
pub struct JobStationVariant(u8);
impl JobStationVariant {
    pub const NONE: JobStationVariant = JobStationVariant(0);
    pub const ENERGY_GENERATOR: JobStationVariant = JobStationVariant(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Zeroable, Pod)]
#[repr(C)]
pub struct ResourceVariant(u8);
impl ResourceVariant {
    pub const NONE: ResourceVariant = ResourceVariant(0);
    pub const MAGMA: ResourceVariant = ResourceVariant(1);
    pub const ENERGY: ResourceVariant = ResourceVariant(2);
}
