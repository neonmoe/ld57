use core::{
    fmt::{self, Debug},
    ops::Deref,
};

use bytemuck::{Pod, Zeroable};
use engine::impl_game_object;
use glam::I16Vec2;

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
    pub stockpile_reliant: StockpileReliantTag,
}
impl_game_object! {
    impl GameObject for Resource using components {
        position: TilePosition,
        stockpile: Stockpile,
        stockpile_reliant: StockpileReliantTag,
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
                resource_amount: 3,
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
    pub reserved: u8,
    pub variants: [ResourceVariant; 3],
    pub amounts: [u8; 3],
}
impl Stockpile {
    pub const fn with_resource(
        mut self,
        resource: ResourceVariant,
        amount: u8,
        reserved: bool,
    ) -> Stockpile {
        let i = self.variant_count as usize;
        if i >= self.variants.len() {
            self
        } else {
            self.variants[i] = resource;
            self.amounts[i] = amount;
            self.reserved |= (reserved as u8) << i;
            self.variant_count += 1;
            self
        }
    }

    /// Adds the resources to this stockpile. If it can't fit, returns the
    /// overflowed amount.
    pub fn add_resource(&mut self, variant: ResourceVariant, amount: u8) -> Result<(), u8> {
        if let Some(existing_amount) = self.get_resources_mut(variant) {
            let capacity_left = u8::MAX - *existing_amount;
            if let Some(overflow) = amount.checked_sub(capacity_left) {
                *existing_amount = u8::MAX;
                return Err(overflow);
            }
            *existing_amount += amount;
        } else if self.variant_count as usize == self.variants.len() {
            let non_reserved_empty_slot =
                |(i, amount)| ((self.reserved >> i as u8) & 0b1) == 0 && amount == 0;
            let Some(empty_idx) = self
                .amounts
                .into_iter()
                .enumerate()
                .position(non_reserved_empty_slot)
            else {
                return Err(amount);
            };
            self.variants[empty_idx] = variant;
            self.amounts[empty_idx] = amount;
        } else {
            *self = self.with_resource(variant, amount, false);
        }
        Ok(())
    }

    pub fn mark_reserved(&mut self, variant: ResourceVariant, reserved: bool) {
        let len = self.variant_count as usize;
        for (i, variant_) in self.variants[..len].iter().enumerate() {
            if variant == *variant_ {
                if reserved {
                    self.reserved |= 0b1 << (i as u8);
                } else {
                    self.reserved &= !(0b1 << (i as u8));
                }
            }
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

    pub fn get_resources(&self, variant: ResourceVariant) -> Option<u8> {
        let len = self.variant_count as usize;
        for (variant_, amount) in self.variants[..len].iter().zip(&self.amounts[..len]) {
            if variant == *variant_ {
                return Some(*amount);
            }
        }
        None
    }

    pub fn has_non_reserved_resources(&self, variant: ResourceVariant) -> bool {
        let len = self.variant_count as usize;
        for (i, (variant_, amount)) in self.variants[..len]
            .iter()
            .zip(&self.amounts[..len])
            .enumerate()
        {
            if variant == *variant_ && ((self.reserved >> i as u8) & 0b1) == 0 {
                return *amount > 0;
            }
        }
        false
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

    pub fn is_empty(self) -> bool {
        for amount in &self.amounts[..self.variant_count as usize] {
            if *amount > 0 {
                return false;
            }
        }
        true
    }
}

#[derive(Clone, Copy, Debug, Zeroable, Pod)]
#[repr(C)]
pub struct StockpileReliantTag;

#[derive(Clone, Copy, PartialEq, Eq, Zeroable, Pod)]
#[repr(C)]
pub struct TilePosition(pub I16Vec2);
impl TilePosition {
    pub fn new(x: i16, y: i16) -> TilePosition {
        TilePosition(I16Vec2 { x, y })
    }
}
impl Deref for TilePosition {
    type Target = I16Vec2;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl Debug for TilePosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("").field(&self.0.x).field(&self.0.y).finish()
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

macro_rules! define_consts_with_nice_debug {
    ([$const_type:tt] {$($variant:ident: $value:literal),*$(,)?}) => {
        impl $const_type {
            $(pub const $variant: $const_type = $const_type($value);)*
        }
        impl Debug for $const_type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                $(if *self == <$const_type>::$variant {
                    return write!(f, concat!(stringify!($const_type), "::", stringify!($variant)));
                })*
                write!(f, concat!(stringify!($const_type), "(unknown value)"))
            }
        }
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Zeroable, Pod)]
#[repr(C)]
pub struct JobStationVariant(u8);
define_consts_with_nice_debug!([JobStationVariant] {
    ENERGY_GENERATOR: 1,
});

#[derive(Clone, Copy, PartialEq, Eq, Zeroable, Pod)]
#[repr(C)]
pub struct ResourceVariant(u8);
define_consts_with_nice_debug!([ResourceVariant] {
    MAGMA: 1,
    ENERGY: 2,
});
