# IPI Mailbox

M5 PR9A separates logical inter-processor messages from the architecture
doorbell used to notify a CPU.

## Model

Every logical CPU owns one mailbox with:

- pending message bits;
- hardware interrupt count;
- doorbells sent;
- coalesced publications;
- handled drain batches;
- spurious hardware interrupts.

The currently supported messages are `Reschedule` and `TlbShootdown`.

## Publication

```text
pending.fetch_or(message, Release)
        |
        +-- previous == 0: own and ring one hardware doorbell
        |
        +-- previous != 0: coalesce into the existing drain
```

Repeated reschedule requests do not require one hardware interrupt each. They
only require the target CPU to observe `need_resched`.

## Drain

The handler acknowledges the architecture doorbell, repeatedly swaps pending
messages to zero, processes TLB invalidation before rescheduling, and continues
until the mailbox is empty.

A publisher racing after the final empty swap sees an empty mailbox and sends a
new doorbell.

An empty mailbox after a valid hardware acknowledgement is legal: another
handler pass may already have consumed the logical message. This event is
counted for diagnostics rather than treated as corruption.

## Follow-ups

PR9A leaves the current TLB generation/ACK protocol unchanged.

- PR9B adds preallocated payload/request slots and call-function messages.
- PR9C moves TLB work to an explicit request object with request ID, target and
  completion masks, plus full/page/range kinds.
