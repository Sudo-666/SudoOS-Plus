# IPI Mailbox

M5 PR9 separates logical messages from architecture doorbells. The first publication into an empty mailbox rings hardware; later publications coalesce until drain.

Messages are `Reschedule`, `TlbShootdown`, and `CallFunction`. A handler processes TLB invalidation, payload callbacks, then rescheduling. `CallFunction` payloads live in preallocated request slots; its mailbox bit is only the doorbell-level notification.
