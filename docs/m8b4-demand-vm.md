# M8-B4 demand paging and VM syscall integration

M8-B4 connects the architecture-neutral B2 fault planner to the private hardware
roots proved by B3. The gate intentionally keeps process scheduling out of scope:
the current verifier still enters one private mm synchronously on the boot CPU.

## Fault lifecycle

A user page fault is handled as follows:

1. decode the fault address/access and saved user stack pointer;
2. classify it through `UserAddressSpace::plan_user_fault()`;
3. for anonymous/heap/stack faults, allocate one zeroed backing page;
4. install the leaf PTE in the active private root;
5. release the per-mm lock;
6. invalidate the exact ASID/page locally;
7. return without advancing the user PC so the instruction is retried.

COW, file-backed, and device-backed user faults remain explicit unsupported
classifications. Protection failures and unmapped addresses terminate only the
current verifier session and do not panic the kernel.

## VMA transactions

`munmap()` and `mprotect()` rebuild fixed-capacity VMA metadata off to the side
and publish it only after every fragment validates. `munmap()` accepts holes,
while `mprotect()` requires complete VMA coverage. Adjacent areas with identical
flags and kind are coalesced so repeated split/protect operations do not consume
VMA capacity indefinitely.

`brk()` rebuilds one contiguous heap VMA transactionally. Shrinking the break
retires mapped heap pages above the new end.

## TLB and reclamation order

B4 follows the same lifetime rule as Linux's `mmu_gather` family: a physical
page or page-table page is not reusable until stale translations are gone.

1. validate the operation and reserve retirement metadata;
2. advance the mm TLB generation and snapshot `active_cpus`;
3. detach leaf PTEs and empty private page-table pages under the per-mm lock;
4. release the per-mm lock;
5. invalidate the exact ASID/range;
6. free retired backing and page-table pages.

The synchronous verifier disables local interrupts while the private root is
active. Therefore B4 adds a deliberately strict local-only per-mm invalidation
helper. It accepts an empty target mask or the caller CPU only. A future mm that
can execute on several CPUs must use the existing remote serializer/ACK path
from interruptible task context.

## ABI boundary

The verifier uses Linux asm-generic syscall numbers:

- `brk = 214`
- `munmap = 215`
- `mmap = 222`
- `mprotect = 226`

This gate supports private anonymous `mmap`, page-rounded lengths, page-aligned
`munmap`/`mprotect`, and W^X. Execute permission implies read permission on both
currently supported architectures. `MAP_FIXED`, file mappings, `PROT_NONE`, COW,
and shared mappings remain later work.
