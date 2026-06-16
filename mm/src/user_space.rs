use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    AddressSpace, AddressSpaceError, AsidToken, AtomicCpuMask, CpuMask, CpuMaskError, FaultOutcome,
    PAGE_SIZE, PageFault, TlbFlush, TlbScope, VirtAddr, VirtRange, VmArea, VmAreaFlags, VmAreaKind,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserMmError {
    AddressSpace(AddressSpaceError),
    CpuMask(CpuMaskError),
    AsidMismatch,
    TlbGenerationMismatch { expected: u64, observed: u64 },
    TlbGenerationOverflow,
    AddressOverflow,
    NotStack,
    StackGrowthDenied,
    ActiveOnCpu,
}

impl From<AddressSpaceError> for UserMmError {
    fn from(error: AddressSpaceError) -> Self {
        Self::AddressSpace(error)
    }
}

impl From<CpuMaskError> for UserMmError {
    fn from(error: CpuMaskError) -> Self {
        Self::CpuMask(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerMmTlbRequest {
    asid: AsidToken,
    targets: CpuMask,
    flush: TlbFlush,
    generation: u64,
}

impl PerMmTlbRequest {
    pub const fn asid(self) -> AsidToken {
        self.asid
    }

    pub const fn targets(self) -> CpuMask {
        self.targets
    }

    pub const fn flush(self) -> TlbFlush {
        self.flush
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StackGrowth {
    old_area: VmArea,
    new_area: VmArea,
    fault_page: VirtAddr,
}

impl StackGrowth {
    pub const fn old_area(self) -> VmArea {
        self.old_area
    }

    pub const fn new_area(self) -> VmArea {
        self.new_area
    }

    pub const fn fault_page(self) -> VirtAddr {
        self.fault_page
    }
}

/// Conservative defaults used by the architecture-neutral user-fault planner.
pub const DEFAULT_STACK_GUARD_GAP: usize = PAGE_SIZE;
pub const DEFAULT_STACK_GROWTH_STEP: usize = PAGE_SIZE * 8;
pub const DEFAULT_STACK_SP_DISTANCE: usize = PAGE_SIZE * 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserFaultPlan {
    MapAnonymous { area: VmArea, page: VirtAddr },
    GrowStack { growth: StackGrowth },
    CopyOnWriteUnsupported { area: VmArea },
    ProtectionViolation { area: VmArea },
    SegmentationViolation,
    Spurious { area: VmArea },
    KernelBug,
}

impl UserFaultPlan {
    pub const fn requires_page_table_update(self) -> bool {
        matches!(
            self,
            Self::MapAnonymous { .. }
                | Self::GrowStack { .. }
                | Self::CopyOnWriteUnsupported { .. }
        )
    }
}

/// Architecture-neutral half of Linux's `mm_struct` contract.
///
/// The kernel wrapper owns the page-table root and page-table lock. This object
/// owns VMA metadata, the generation-tagged ASID, `mm_cpumask` equivalent, and
/// the per-mm TLB sequence number. Keeping hardware ownership out of this crate
/// makes all state-machine rules host-testable.
pub struct UserAddressSpace<const VMA_CAPACITY: usize> {
    layout: AddressSpace<VMA_CAPACITY>,
    asid: AsidToken,
    active_cpus: AtomicCpuMask,
    tlb_generation: AtomicU64,
}

impl<const VMA_CAPACITY: usize> UserAddressSpace<VMA_CAPACITY> {
    pub const fn new(user_range: VirtRange, asid: AsidToken) -> Self {
        Self {
            layout: AddressSpace::new(user_range),
            asid,
            active_cpus: AtomicCpuMask::new(CpuMask::EMPTY),
            tlb_generation: AtomicU64::new(0),
        }
    }

    pub const fn asid(&self) -> AsidToken {
        self.asid
    }

    pub const fn layout(&self) -> &AddressSpace<VMA_CAPACITY> {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut AddressSpace<VMA_CAPACITY> {
        &mut self.layout
    }

    pub fn tlb_generation(&self) -> u64 {
        self.tlb_generation.load(Ordering::SeqCst)
    }

    /// Publish this CPU only after installing the hardware root and bringing
    /// its local ASID state to `synchronized_tlb_generation`.
    ///
    /// Insertion happens before the final generation check. Therefore a racing
    /// invalidation either targets this CPU or makes this call fail so the
    /// switch path can remove the bit, flush locally, and retry.
    pub fn enter_cpu_after_local_sync(
        &self,
        cpu: usize,
        current_asid_generation: u64,
        synchronized_tlb_generation: u64,
    ) -> Result<(), UserMmError> {
        if !self.asid.is_current(current_asid_generation) {
            return Err(UserMmError::AsidMismatch);
        }

        self.active_cpus.insert(cpu, Ordering::SeqCst)?;
        let observed = self.tlb_generation();
        if observed != synchronized_tlb_generation {
            self.active_cpus.remove(cpu, Ordering::SeqCst)?;
            return Err(UserMmError::TlbGenerationMismatch {
                expected: synchronized_tlb_generation,
                observed,
            });
        }
        Ok(())
    }

    /// Clear this CPU only after the next root is active and the departed ASID
    /// has been invalidated locally. A stale generation keeps the CPU published
    /// so a caller cannot accidentally escape a concurrent shootdown.
    pub fn leave_cpu_after_local_flush(
        &self,
        cpu: usize,
        flushed_tlb_generation: u64,
    ) -> Result<(), UserMmError> {
        let observed = self.tlb_generation();
        if observed != flushed_tlb_generation {
            return Err(UserMmError::TlbGenerationMismatch {
                expected: flushed_tlb_generation,
                observed,
            });
        }
        self.active_cpus.remove(cpu, Ordering::SeqCst)?;
        Ok(())
    }

    pub fn active_cpus(&self) -> CpuMask {
        self.active_cpus.load(Ordering::SeqCst)
    }

    pub fn assert_inactive_for_destroy(&self) -> Result<(), UserMmError> {
        if self.active_cpus().is_empty() {
            Ok(())
        } else {
            Err(UserMmError::ActiveOnCpu)
        }
    }

    pub fn map_area(&mut self, area: VmArea) -> Result<(), UserMmError> {
        self.layout.map_area(area)?;
        Ok(())
    }

    pub fn unmap_exact(&mut self, range: VirtRange) -> Result<VmArea, UserMmError> {
        Ok(self.layout.unmap_exact(range)?)
    }

    /// Exact-range mprotect is transactional at the VMA layer: if inserting
    /// the replacement fails, the original area is restored before returning.
    pub fn protect_exact(
        &mut self,
        range: VirtRange,
        flags: VmAreaFlags,
    ) -> Result<VmArea, UserMmError> {
        let old = self.layout.unmap_exact(range)?;
        let replacement = VmArea::new(range, flags, old.kind());
        if let Err(error) = self.layout.map_area(replacement) {
            self.layout
                .map_area(old)
                .expect("mprotect rollback could not restore original VMA");
            return Err(UserMmError::AddressSpace(error));
        }
        Ok(old)
    }

    pub fn resolve_fault(&self, fault: PageFault) -> FaultOutcome {
        fault.resolve(&self.layout)
    }

    /// Bounded Linux-like GROWSDOWN policy. Growth is accepted only when:
    /// - the candidate is a user stack with GROW_DOWN;
    /// - saved user SP belongs to that stack (or equals its end boundary);
    /// - the fault is within one bounded growth step and close to saved SP;
    /// - the expanded VMA preserves a guard gap above the preceding VMA;
    /// - the expanded range remains inside the configured user range.

    pub fn plan_user_fault(
        &self,
        fault: PageFault,
        user_sp: VirtAddr,
    ) -> Result<UserFaultPlan, UserMmError> {
        self.plan_user_fault_with_limits(
            fault,
            user_sp,
            DEFAULT_STACK_GUARD_GAP,
            DEFAULT_STACK_GROWTH_STEP,
            DEFAULT_STACK_SP_DISTANCE,
        )
    }

    pub fn plan_user_fault_with_limits(
        &self,
        fault: PageFault,
        user_sp: VirtAddr,
        stack_guard_gap: usize,
        max_growth_step: usize,
        max_sp_distance: usize,
    ) -> Result<UserFaultPlan, UserMmError> {
        if fault.source() != crate::FaultSource::User {
            return Ok(UserFaultPlan::KernelBug);
        }

        match self.resolve_fault(fault) {
            FaultOutcome::MapAnonymous { area } => Ok(UserFaultPlan::MapAnonymous {
                area,
                page: fault
                    .address()
                    .align_down(PAGE_SIZE)
                    .ok_or(UserMmError::AddressOverflow)?,
            }),
            FaultOutcome::CopyOnWrite { area } => {
                Ok(UserFaultPlan::CopyOnWriteUnsupported { area })
            }
            FaultOutcome::LoadFile { .. } | FaultOutcome::MapDevice { .. } => {
                Ok(UserFaultPlan::KernelBug)
            }
            FaultOutcome::ProtectionViolation { area } => {
                Ok(UserFaultPlan::ProtectionViolation { area })
            }
            FaultOutcome::Spurious { area } => Ok(UserFaultPlan::Spurious { area }),
            FaultOutcome::KernelBug => Ok(UserFaultPlan::KernelBug),
            FaultOutcome::SegmentationViolation => {
                match self.plan_stack_growth(
                    fault.address(),
                    user_sp,
                    stack_guard_gap,
                    max_growth_step,
                    max_sp_distance,
                ) {
                    Ok(growth) => Ok(UserFaultPlan::GrowStack { growth }),
                    Err(UserMmError::StackGrowthDenied) => Ok(UserFaultPlan::SegmentationViolation),
                    Err(error) => Err(error),
                }
            }
        }
    }

    pub fn plan_post_install_tlb(&self, page: VirtAddr) -> Result<PerMmTlbRequest, UserMmError> {
        let page = page
            .align_down(PAGE_SIZE)
            .ok_or(UserMmError::AddressOverflow)?;
        self.plan_tlb_request(TlbFlush::Page {
            scope: TlbScope::AddressSpace(self.asid.id()),
            address: page,
        })
    }

    pub fn plan_stack_growth(
        &self,
        fault: VirtAddr,
        user_sp: VirtAddr,
        stack_guard_gap: usize,
        max_growth_step: usize,
        max_sp_distance: usize,
    ) -> Result<StackGrowth, UserMmError> {
        let fault_page = fault
            .align_down(PAGE_SIZE)
            .ok_or(UserMmError::AddressOverflow)?;

        for index in 0..self.layout.area_count() {
            let area = self
                .layout
                .area_at(index)
                .expect("area below AddressSpace::area_count is missing");
            if area.kind() != VmAreaKind::Stack
                || !area.flags().contains(VmAreaFlags::GROW_DOWN)
                || fault_page >= area.range().start()
            {
                continue;
            }

            let sp_matches = area.range().contains(user_sp) || user_sp == area.range().end();
            if !sp_matches {
                continue;
            }

            let step_limit = area
                .range()
                .start()
                .checked_sub(max_growth_step)
                .ok_or(UserMmError::AddressOverflow)?;
            if fault_page < step_limit {
                continue;
            }

            let sp_limit = user_sp
                .checked_sub(max_sp_distance)
                .ok_or(UserMmError::AddressOverflow)?;
            if fault < sp_limit {
                continue;
            }

            if index > 0 {
                let previous = self
                    .layout
                    .area_at(index - 1)
                    .expect("previous VMA below AddressSpace::area_count is missing");
                let minimum_start = previous
                    .range()
                    .end()
                    .checked_add(stack_guard_gap)
                    .ok_or(UserMmError::AddressOverflow)?;
                if fault_page < minimum_start {
                    continue;
                }
            }

            let new_range = VirtRange::new(fault_page, area.range().end())
                .ok_or(UserMmError::AddressOverflow)?;
            if !self.layout.user_range().contains_range(new_range) {
                continue;
            }

            return Ok(StackGrowth {
                old_area: area,
                new_area: VmArea::new(new_range, area.flags(), area.kind()),
                fault_page,
            });
        }

        Err(UserMmError::StackGrowthDenied)
    }

    pub fn commit_stack_growth(&mut self, plan: StackGrowth) -> Result<(), UserMmError> {
        let removed = self.layout.unmap_exact(plan.old_area().range())?;
        if removed != plan.old_area() {
            self.layout
                .map_area(removed)
                .expect("stack-growth rollback could not restore VMA");
            return Err(UserMmError::NotStack);
        }
        if let Err(error) = self.layout.map_area(plan.new_area()) {
            self.layout
                .map_area(removed)
                .expect("stack-growth rollback could not restore VMA");
            return Err(UserMmError::AddressSpace(error));
        }
        Ok(())
    }

    /// Build a per-mm request from a snapshot of `active_cpus`. The kernel must
    /// not hold its page-table lock while waiting for remote acknowledgements.
    pub fn plan_tlb_request(&self, flush: TlbFlush) -> Result<PerMmTlbRequest, UserMmError> {
        let expected_scope = TlbScope::AddressSpace(self.asid.id());
        let scope_matches = match flush {
            TlbFlush::All { scope }
            | TlbFlush::Page { scope, .. }
            | TlbFlush::Range { scope, .. } => scope == expected_scope,
        };
        if !scope_matches {
            return Err(UserMmError::AsidMismatch);
        }

        let generation = self.next_tlb_generation()?;
        Ok(PerMmTlbRequest {
            asid: self.asid,
            targets: self.active_cpus(),
            flush,
            generation,
        })
    }

    fn next_tlb_generation(&self) -> Result<u64, UserMmError> {
        let mut current = self.tlb_generation.load(Ordering::SeqCst);
        loop {
            let next = current
                .checked_add(1)
                .ok_or(UserMmError::TlbGenerationOverflow)?;
            match self.tlb_generation.compare_exchange_weak(
                current,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Ok(next),
                Err(observed) => current = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AddressSpaceId, FaultAccess, FaultSource};

    const USER_RANGE: VirtRange = VirtRange::from_bounds(0x1000, 0x20_0000);

    fn token() -> AsidToken {
        AsidToken::new(AddressSpaceId::new(7), 3)
    }

    #[test]
    fn tracks_active_cpus_and_scopes_tlb_requests() {
        let mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        mm.enter_cpu_after_local_sync(1, 3, 0).unwrap();
        mm.enter_cpu_after_local_sync(4, 3, 0).unwrap();
        let request = mm
            .plan_tlb_request(TlbFlush::All {
                scope: TlbScope::AddressSpace(AddressSpaceId::new(7)),
            })
            .unwrap();
        assert!(request.targets().contains(1).unwrap());
        assert!(request.targets().contains(4).unwrap());
        assert_eq!(request.generation(), 1);
        assert_eq!(
            mm.plan_tlb_request(TlbFlush::All {
                scope: TlbScope::AddressSpace(AddressSpaceId::new(8)),
            }),
            Err(UserMmError::AsidMismatch)
        );
    }

    #[test]
    fn rejects_stale_asid_on_cpu_entry() {
        let mm: UserAddressSpace<4> = UserAddressSpace::new(USER_RANGE, token());
        assert_eq!(
            mm.enter_cpu_after_local_sync(0, 4, 0),
            Err(UserMmError::AsidMismatch)
        );
        assert!(mm.active_cpus().is_empty());
    }

    #[test]
    fn generation_handshake_closes_reentry_shootdown_hole() {
        let mm: UserAddressSpace<4> = UserAddressSpace::new(USER_RANGE, token());
        let request = mm
            .plan_tlb_request(TlbFlush::All {
                scope: TlbScope::AddressSpace(AddressSpaceId::new(7)),
            })
            .unwrap();
        assert_eq!(request.generation(), 1);
        assert_eq!(
            mm.enter_cpu_after_local_sync(2, 3, 0),
            Err(UserMmError::TlbGenerationMismatch {
                expected: 0,
                observed: 1,
            })
        );
        assert!(mm.active_cpus().is_empty());

        mm.enter_cpu_after_local_sync(2, 3, 1).unwrap();
        assert_eq!(
            mm.leave_cpu_after_local_flush(2, 0),
            Err(UserMmError::TlbGenerationMismatch {
                expected: 0,
                observed: 1,
            })
        );
        assert!(mm.active_cpus().contains(2).unwrap());
        mm.leave_cpu_after_local_flush(2, 1).unwrap();
        assert!(mm.active_cpus().is_empty());
    }

    #[test]
    fn plans_and_commits_bounded_stack_growth() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let flags = VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN);
        let stack = VmArea::new(
            VirtRange::from_bounds(0x10_0000, 0x11_0000),
            flags,
            VmAreaKind::Stack,
        );
        mm.map_area(stack).unwrap();

        let plan = mm
            .plan_stack_growth(
                VirtAddr::new(0x0f_f123),
                VirtAddr::new(0x10_1000),
                PAGE_SIZE,
                PAGE_SIZE * 4,
                PAGE_SIZE * 4,
            )
            .unwrap();
        assert_eq!(plan.fault_page(), VirtAddr::new(0x0f_f000));
        mm.commit_stack_growth(plan).unwrap();
        assert!(mm.layout().find_area(VirtAddr::new(0x0f_f100)).is_some());
    }

    #[test]
    fn stack_growth_preserves_gap_above_previous_vma() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        mm.map_area(VmArea::new(
            VirtRange::from_bounds(0x0f_d000, 0x0f_e000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        ))
        .unwrap();
        mm.map_area(VmArea::new(
            VirtRange::from_bounds(0x10_0000, 0x11_0000),
            VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
            VmAreaKind::Stack,
        ))
        .unwrap();

        assert_eq!(
            mm.plan_stack_growth(
                VirtAddr::new(0x0f_e123),
                VirtAddr::new(0x10_1000),
                PAGE_SIZE * 2,
                PAGE_SIZE * 4,
                PAGE_SIZE * 4,
            ),
            Err(UserMmError::StackGrowthDenied)
        );
    }

    #[test]
    fn denies_far_stack_jump() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let stack = VmArea::new(
            VirtRange::from_bounds(0x10_0000, 0x11_0000),
            VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
            VmAreaKind::Stack,
        );
        mm.map_area(stack).unwrap();
        assert_eq!(
            mm.plan_stack_growth(
                VirtAddr::new(0x0f_0000),
                VirtAddr::new(0x10_1000),
                PAGE_SIZE,
                PAGE_SIZE * 4,
                PAGE_SIZE * 4,
            ),
            Err(UserMmError::StackGrowthDenied)
        );
    }

    #[test]
    fn plans_anonymous_demand_fault_and_post_install_tlb() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let area = VmArea::new(
            VirtRange::from_bounds(0x4000, 0x6000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        );
        mm.map_area(area).unwrap();
        mm.enter_cpu_after_local_sync(0, 3, 0).unwrap();

        let fault = PageFault::new(
            VirtAddr::new(0x4123),
            FaultAccess::Write,
            FaultSource::User,
            false,
        );
        assert_eq!(
            mm.plan_user_fault(fault, VirtAddr::new(0x10_1000)).unwrap(),
            UserFaultPlan::MapAnonymous {
                area,
                page: VirtAddr::new(0x4000),
            }
        );

        let request = mm.plan_post_install_tlb(VirtAddr::new(0x4123)).unwrap();
        assert_eq!(request.generation(), 1);
        assert_eq!(
            request.flush(),
            TlbFlush::Page {
                scope: TlbScope::AddressSpace(AddressSpaceId::new(7)),
                address: VirtAddr::new(0x4000),
            }
        );
        assert!(request.targets().contains(0).unwrap());
    }

    #[test]
    fn grows_stack_before_declaring_segv() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let old = VmArea::new(
            VirtRange::from_bounds(0x10_0000, 0x11_0000),
            VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
            VmAreaKind::Stack,
        );
        mm.map_area(old).unwrap();
        let fault = PageFault::new(
            VirtAddr::new(0x0f_f800),
            FaultAccess::Write,
            FaultSource::User,
            false,
        );
        match mm.plan_user_fault(fault, VirtAddr::new(0x10_1000)).unwrap() {
            UserFaultPlan::GrowStack { growth } => {
                assert_eq!(growth.old_area(), old);
                assert_eq!(growth.fault_page(), VirtAddr::new(0x0f_f000));
            }
            other => panic!("expected stack growth, got {other:?}"),
        }
    }

    #[test]
    fn rejects_far_stack_fault_as_segv() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        mm.map_area(VmArea::new(
            VirtRange::from_bounds(0x10_0000, 0x11_0000),
            VmAreaFlags::user_rw().union(VmAreaFlags::GROW_DOWN),
            VmAreaKind::Stack,
        ))
        .unwrap();
        let fault = PageFault::new(
            VirtAddr::new(0x0e_0000),
            FaultAccess::Write,
            FaultSource::User,
            false,
        );
        assert_eq!(
            mm.plan_user_fault(fault, VirtAddr::new(0x10_1000)).unwrap(),
            UserFaultPlan::SegmentationViolation,
        );
    }

    #[test]
    fn keeps_cow_explicit_until_fork_stage() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let flags = VmAreaFlags::READ
            .union(VmAreaFlags::USER)
            .union(VmAreaFlags::PRIVATE)
            .union(VmAreaFlags::COPY_ON_WRITE);
        let area = VmArea::new(
            VirtRange::from_bounds(0x7000, 0x8000),
            flags,
            VmAreaKind::Anonymous,
        );
        mm.map_area(area).unwrap();
        let fault = PageFault::new(
            VirtAddr::new(0x7123),
            FaultAccess::Write,
            FaultSource::User,
            true,
        );
        assert_eq!(
            mm.plan_user_fault(fault, VirtAddr::new(0x10_1000)).unwrap(),
            UserFaultPlan::CopyOnWriteUnsupported { area },
        );
    }

    #[test]
    fn existing_fault_policy_remains_usable() {
        let mut mm: UserAddressSpace<8> = UserAddressSpace::new(USER_RANGE, token());
        let area = VmArea::new(
            VirtRange::from_bounds(0x4000, 0x5000),
            VmAreaFlags::user_rw(),
            VmAreaKind::Anonymous,
        );
        mm.map_area(area).unwrap();
        let fault = PageFault::new(
            VirtAddr::new(0x4123),
            FaultAccess::Write,
            FaultSource::User,
            false,
        );
        assert_eq!(mm.resolve_fault(fault), FaultOutcome::MapAnonymous { area });
    }
}
