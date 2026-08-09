use alloc::vec::Vec;

use crate::AddressSpaceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsidToken {
    id: AddressSpaceId,
    generation: u64,
}

impl AsidToken {
    pub const fn new(id: AddressSpaceId, generation: u64) -> Self {
        Self { id, generation }
    }

    pub const fn id(self) -> AddressSpaceId {
        self.id
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn is_current(self, generation: u64) -> bool {
        self.generation == generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AsidAllocation {
    token: AsidToken,
    generation_rolled: bool,
}

impl AsidAllocation {
    pub const fn token(self) -> AsidToken {
        self.token
    }

    /// A rollover requires a global user-ASID TLB invalidation before the
    /// returned token may become visible to hardware.
    pub const fn generation_rolled(self) -> bool {
        self.generation_rolled
    }

    pub const fn requires_global_flush(self) -> bool {
        self.generation_rolled
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AsidAllocatorError {
    NoUserAsids,
    GenerationOverflow,
}

/// Linux-like generation allocator. ASID 0 is permanently reserved for the
/// shared kernel address space. Callers serialize this object with the kernel's
/// MM/ASID lock and perform the rollover flush outside that lock.
#[derive(Debug)]
pub struct AsidAllocator {
    maximum: u16,
    // One bit wider than a hardware ASID so maximum == u16::MAX can advance
    // to 65536 without wrapping through the reserved kernel ASID 0.
    next: u32,
    generation: u64,
    free: Vec<u16>,
}

impl AsidAllocator {
    pub const fn new(maximum: u16) -> Result<Self, AsidAllocatorError> {
        if maximum == 0 {
            return Err(AsidAllocatorError::NoUserAsids);
        }
        Ok(Self {
            maximum,
            next: 1,
            generation: 1,
            free: Vec::new(),
        })
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn maximum(&self) -> u16 {
        self.maximum
    }

    /// True when the next allocation would recycle hardware IDs into a new
    /// generation. M8 uses this to fail closed while any old-generation MM is
    /// still alive; M9 may replace that gate with lazy Linux-style renewal.
    pub fn next_allocation_rolls_generation(&self) -> bool {
        self.free.is_empty() && self.next > self.maximum as u32
    }

    pub fn allocate(&mut self) -> Result<AsidAllocation, AsidAllocatorError> {
        if let Some(id) = self.free.pop() {
            return Ok(AsidAllocation {
                token: AsidToken::new(AddressSpaceId::new(id), self.generation),
                generation_rolled: false,
            });
        }
        let mut rolled = false;
        if self.next > u32::from(self.maximum) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(AsidAllocatorError::GenerationOverflow)?;
            self.next = 1;
            rolled = true;
        }

        let id = u16::try_from(self.next).expect("ASID cursor exceeds u16 after rollover check");
        self.next += 1;
        Ok(AsidAllocation {
            token: AsidToken::new(AddressSpaceId::new(id), self.generation),
            generation_rolled: rolled,
        })
    }

    /// Return an inactive MM's hardware ID for reuse in the same generation.
    /// Destruction is permitted only after every CPU has left the MM and
    /// flushed this ASID locally, so the returned ID is immediately reusable.
    pub fn release(&mut self, token: AsidToken) {
        if !token.is_current(self.generation) || token.id() == AddressSpaceId::KERNEL {
            return;
        }
        let id = token.id().get();
        assert!(id <= self.maximum, "released ASID exceeds allocator maximum");
        assert!(!self.free.contains(&id), "ASID released twice");
        self.free.push(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_kernel_asid_and_rolls_generation() {
        let mut allocator = AsidAllocator::new(2).unwrap();
        let first = allocator.allocate().unwrap();
        let second = allocator.allocate().unwrap();
        let third = allocator.allocate().unwrap();

        assert_eq!(first.token().id().get(), 1);
        assert_eq!(second.token().id().get(), 2);
        assert!(!first.generation_rolled());
        assert!(third.generation_rolled());
        assert_eq!(third.token().id().get(), 1);
        assert_eq!(third.token().generation(), first.token().generation() + 1);
    }

    #[test]
    fn maximum_hardware_asid_never_wraps_through_zero() {
        let mut allocator = AsidAllocator::new(u16::MAX).unwrap();
        allocator.next = u32::from(u16::MAX);

        let last = allocator.allocate().unwrap();
        let rolled = allocator.allocate().unwrap();

        assert_eq!(last.token().id().get(), u16::MAX);
        assert_eq!(rolled.token().id().get(), 1);
        assert!(rolled.generation_rolled());
    }

    #[test]
    fn reports_the_rollover_boundary_before_reusing_an_id() {
        let mut allocator = AsidAllocator::new(1).unwrap();
        assert!(!allocator.next_allocation_rolls_generation());
        allocator.allocate().unwrap();
        assert!(allocator.next_allocation_rolls_generation());
    }

    #[test]
    fn rejects_allocator_without_user_asids() {
        assert!(matches!(
            AsidAllocator::new(0),
            Err(AsidAllocatorError::NoUserAsids)
        ));
    }
}
