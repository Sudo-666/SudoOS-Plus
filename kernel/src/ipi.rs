use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering, fence};

use crate::smp::{CpuId, MAX_CPUS};

const IPI_RESCHEDULE: usize = 1 << 0;
const IPI_TLB_SHOOTDOWN: usize = 1 << 1;
const IPI_CALL_FUNCTION: usize = 1 << 2;
const IPI_KNOWN_MASK: usize = IPI_RESCHEDULE | IPI_TLB_SHOOTDOWN | IPI_CALL_FUNCTION;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpiMessage {
    Reschedule,
    TlbShootdown,
    CallFunction,
}

impl IpiMessage {
    const fn bit(self) -> usize {
        match self {
            Self::Reschedule => IPI_RESCHEDULE,
            Self::TlbShootdown => IPI_TLB_SHOOTDOWN,
            Self::CallFunction => IPI_CALL_FUNCTION,
        }
    }
}

struct IpiMailbox {
    pending: AtomicUsize,
    interrupts: AtomicU64,
    doorbells: AtomicU64,
    coalesced: AtomicU64,
    handled_batches: AtomicU64,
    spurious_interrupts: AtomicU64,
}

impl IpiMailbox {
    const fn new() -> Self {
        Self {
            pending: AtomicUsize::new(0),
            interrupts: AtomicU64::new(0),
            doorbells: AtomicU64::new(0),
            coalesced: AtomicU64::new(0),
            handled_batches: AtomicU64::new(0),
            spurious_interrupts: AtomicU64::new(0),
        }
    }

    fn reset(&self) {
        self.pending.store(0, Ordering::Release);
        self.interrupts.store(0, Ordering::Release);
        self.doorbells.store(0, Ordering::Release);
        self.coalesced.store(0, Ordering::Release);
        self.handled_batches.store(0, Ordering::Release);
        self.spurious_interrupts.store(0, Ordering::Release);
    }

    /// Publishes one level-triggered message.
    ///
    /// Returning true transfers doorbell ownership to the caller. Returning
    /// false means an already-pending doorbell owns delivery and this message
    /// was coalesced into the same drain pass.
    fn publish(&self, message: IpiMessage) -> bool {
        let bit = message.bit();
        let previous = self.pending.fetch_or(bit, Ordering::Release);

        if previous == 0 {
            self.doorbells.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            self.coalesced.fetch_add(1, Ordering::Relaxed);
            false
        }
    }

    fn take_pending(&self) -> usize {
        self.pending.swap(0, Ordering::AcqRel)
    }
}

static MAILBOXES: [IpiMailbox; MAX_CPUS] = [const { IpiMailbox::new() }; MAX_CPUS];

pub fn initialize() {
    // Initialization runs before secondary CPUs are started, so resetting the
    // mailboxes does not race with publishers or handlers.
    for mailbox in &MAILBOXES {
        mailbox.reset();
    }
    crate::call_function::initialize();
}

/// Publishes a message to one CPU and rings its hardware doorbell only when
/// this publication transitions the mailbox from empty to non-empty.
pub fn send(cpu: CpuId, message: IpiMessage) {
    assert!(
        crate::smp::is_online(cpu),
        "attempted to send an IPI to an offline CPU",
    );
    assert!(
        crate::smp::is_ipi_ready(cpu),
        "attempted to send an IPI to a CPU that is not IPI-ready: cpu={}",
        cpu.get(),
    );

    let mailbox = &MAILBOXES[cpu.get()];
    if !mailbox.publish(message) {
        return;
    }

    // The message publication must be globally visible before the
    // architecture-specific doorbell is observed by the target CPU.
    fence(Ordering::SeqCst);

    let hardware = crate::smp::hardware_id(cpu);
    crate::arch::smp::send_ipi(hardware).unwrap_or_else(|error| {
        dump();
        panic!(
            "unable to ring IPI doorbell: logical={} hardware={} \
             message={message:?} error={error:?}",
            cpu.get(),
            hardware,
        );
    });
}

/// Acknowledges one hardware doorbell and drains every message that was
/// published before or during the drain.
///
/// A second doorbell may legitimately become visible after a message was
/// already consumed by the first handler pass. Such an empty hardware
/// interrupt is counted as spurious rather than treated as corruption.
pub fn handle_current() {
    let cpu = crate::smp::current_cpu_id();
    let hardware_action = crate::arch::smp::acknowledge_ipi();
    assert!(
        hardware_action != 0,
        "IPI exception arrived without a hardware pending action",
    );

    let mailbox = &MAILBOXES[cpu.get()];
    mailbox.interrupts.fetch_add(1, Ordering::Relaxed);
    fence(Ordering::Acquire);

    let mut handled_any = false;
    loop {
        let messages = mailbox.take_pending();
        if messages == 0 {
            break;
        }

        handled_any = true;
        assert_eq!(
            messages & !IPI_KNOWN_MASK,
            0,
            "mailbox contains an unknown IPI message: cpu={} pending={messages:#x}",
            cpu.get(),
        );

        // Flush before rescheduling: a task woken by this interrupt must not
        // run with stale kernel translations.
        if messages & IPI_TLB_SHOOTDOWN != 0 {
            crate::tlb::handle_shootdown_ipi();
        }
        if messages & IPI_CALL_FUNCTION != 0 {
            crate::call_function::handle_current();
        }
        if messages & IPI_RESCHEDULE != 0 {
            crate::task::request_reschedule_local();
        }

        mailbox.handled_batches.fetch_add(1, Ordering::Relaxed);
    }

    if !handled_any {
        mailbox.spurious_interrupts.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn interrupt_count(cpu: CpuId) -> u64 {
    MAILBOXES[cpu.get()].interrupts.load(Ordering::Acquire)
}

pub fn dump() {
    crate::println!("IPI mailboxes:");
    for logical in 0..crate::smp::discovered_cpu_count() {
        let cpu = CpuId::new(logical).expect("discovered CPU exceeds MAX_CPUS");
        let mailbox = &MAILBOXES[logical];
        crate::println!(
            "  cpu{} pending={:#x} irq={} doorbell={} coalesced={} \
             batches={} spurious={}",
            logical,
            mailbox.pending.load(Ordering::Acquire),
            mailbox.interrupts.load(Ordering::Acquire),
            mailbox.doorbells.load(Ordering::Acquire),
            mailbox.coalesced.load(Ordering::Acquire),
            mailbox.handled_batches.load(Ordering::Acquire),
            mailbox.spurious_interrupts.load(Ordering::Acquire),
        );

        assert!(
            crate::smp::is_ipi_ready(cpu) || mailbox.pending.load(Ordering::Acquire) == 0,
            "non-ready CPU has pending IPI messages: cpu={logical}",
        );
    }
    crate::call_function::dump();
}

#[cfg(debug_assertions)]
pub fn verify() {
    let mailbox = IpiMailbox::new();

    assert!(mailbox.publish(IpiMessage::Reschedule));
    assert!(!mailbox.publish(IpiMessage::Reschedule));
    assert!(!mailbox.publish(IpiMessage::TlbShootdown));
    assert!(!mailbox.publish(IpiMessage::CallFunction));
    assert_eq!(mailbox.doorbells.load(Ordering::Acquire), 1);
    assert_eq!(mailbox.coalesced.load(Ordering::Acquire), 3);

    let messages = mailbox.take_pending();
    assert_eq!(
        messages,
        IPI_RESCHEDULE | IPI_TLB_SHOOTDOWN | IPI_CALL_FUNCTION,
        "IPI mailbox failed to coalesce independent messages",
    );
    assert_eq!(mailbox.take_pending(), 0);

    // Once drained, the next publication owns a fresh doorbell.
    assert!(mailbox.publish(IpiMessage::Reschedule));
    assert_eq!(mailbox.doorbells.load(Ordering::Acquire), 2);

    crate::println!("IPI mailbox test:");
    crate::println!("  empty -> doorbell : verified");
    crate::println!("  pending coalescing: verified");
    crate::println!("  payload message   : verified");
    crate::println!("  drain and re-arm  : verified");
}
