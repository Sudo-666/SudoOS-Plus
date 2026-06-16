use core::sync::atomic::{AtomicU64, Ordering};

pub const CPU_MASK_BITS: usize = u64::BITS as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuMaskError {
    CpuOutOfRange { cpu: usize },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(transparent)]
pub struct CpuMask(u64);

impl CpuMask {
    pub const EMPTY: Self = Self(0);
    pub const ALL: Self = Self(u64::MAX);

    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn count(self) -> usize {
        self.0.count_ones() as usize
    }

    pub fn contains(self, cpu: usize) -> Result<bool, CpuMaskError> {
        Ok(self.0 & cpu_bit(cpu)? != 0)
    }

    pub fn with_cpu(self, cpu: usize) -> Result<Self, CpuMaskError> {
        Ok(Self(self.0 | cpu_bit(cpu)?))
    }

    pub fn without_cpu(self, cpu: usize) -> Result<Self, CpuMaskError> {
        Ok(Self(self.0 & !cpu_bit(cpu)?))
    }

    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub fn first(self) -> Option<usize> {
        if self.is_empty() {
            None
        } else {
            Some(self.0.trailing_zeros() as usize)
        }
    }
}

#[derive(Debug)]
#[repr(transparent)]
pub struct AtomicCpuMask(AtomicU64);

impl AtomicCpuMask {
    pub const fn new(initial: CpuMask) -> Self {
        Self(AtomicU64::new(initial.bits()))
    }

    pub fn load(&self, ordering: Ordering) -> CpuMask {
        CpuMask::from_bits(self.0.load(ordering))
    }

    pub fn store(&self, value: CpuMask, ordering: Ordering) {
        self.0.store(value.bits(), ordering);
    }

    pub fn insert(&self, cpu: usize, ordering: Ordering) -> Result<bool, CpuMaskError> {
        let bit = cpu_bit(cpu)?;
        let previous = self.0.fetch_or(bit, ordering);
        Ok(previous & bit == 0)
    }

    pub fn remove(&self, cpu: usize, ordering: Ordering) -> Result<bool, CpuMaskError> {
        let bit = cpu_bit(cpu)?;
        let previous = self.0.fetch_and(!bit, ordering);
        Ok(previous & bit != 0)
    }

    pub fn clear(&self, ordering: Ordering) -> CpuMask {
        CpuMask::from_bits(self.0.swap(0, ordering))
    }
}

const fn cpu_bit(cpu: usize) -> Result<u64, CpuMaskError> {
    if cpu < CPU_MASK_BITS {
        Ok(1_u64 << cpu)
    } else {
        Err(CpuMaskError::CpuOutOfRange { cpu })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_mask_operations_are_bounded() {
        let mask = CpuMask::EMPTY.with_cpu(1).unwrap().with_cpu(63).unwrap();
        assert!(mask.contains(1).unwrap());
        assert!(mask.contains(63).unwrap());
        assert_eq!(mask.count(), 2);
        assert_eq!(mask.first(), Some(1));
        assert_eq!(
            mask.with_cpu(64),
            Err(CpuMaskError::CpuOutOfRange { cpu: 64 })
        );
    }

    #[test]
    fn atomic_membership_reports_transitions() {
        let mask = AtomicCpuMask::new(CpuMask::EMPTY);
        assert!(mask.insert(3, Ordering::AcqRel).unwrap());
        assert!(!mask.insert(3, Ordering::AcqRel).unwrap());
        assert!(mask.load(Ordering::Acquire).contains(3).unwrap());
        assert!(mask.remove(3, Ordering::AcqRel).unwrap());
        assert!(!mask.remove(3, Ordering::AcqRel).unwrap());
        assert!(mask.load(Ordering::Acquire).is_empty());
    }
}
