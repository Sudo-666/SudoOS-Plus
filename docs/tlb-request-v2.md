# TLB Request v2

M5 PR9C replaces the implicit global generation/per-CPU generation-ACK
protocol with one explicit request object.

## Request lifecycle

```text
Free -> Publishing -> Ready -> Free
```

The global serializer still allows one kernel shootdown at a time. This keeps
M5 simple while making ownership explicit.

A published request contains:

- request ID;
- `myos_mm::TlbFlush`;
- target CPU mask;
- atomic completion mask.

The caller does not release the request slot until every target has completed
its local invalidation.

## Flush kinds

The kernel now implements:

```text
All
Page
Range
```

Both supported architectures provide `flush_page()` and `flush_all()`.

- short ranges invalidate each page;
- ranges above 32 pages fall back to a full local flush;
- empty ranges are valid no-ops;
- page and range requests must be page-aligned.

## Scope

M5 supports:

- `Local`;
- `AllCpus`;
- `AddressSpace(AddressSpaceId::KERNEL)`.

Non-kernel address spaces are rejected. A correct per-process shootdown needs
the address space's active CPU mask, ASID generation and teardown protocol;
those belong to user-MM work after M5.

## Ordering

The request is published with release ordering before TLB mailbox messages are
sent. Remote handlers acquire the request, perform the local invalidation, then
publish their completion bit. The caller observes all completions before page
table pages or backing pages may be reclaimed.

## Compatibility

Existing users of `shootdown_kernel_all()` and the M4C2 counters remain
compatible.

New APIs:

```rust
shootdown_kernel_page(address)
shootdown_kernel_range(range)
shootdown(flush)
```
