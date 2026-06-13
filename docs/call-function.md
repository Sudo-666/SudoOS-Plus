# Cross-CPU Call Function

PR9B provides synchronous cross-CPU callbacks without heap allocation. `MAX_CPUS` static request slots transition `Free -> Reserved -> Ready -> Free`; each CPU owns a pending-slot bitmap.

APIs: `smp::call_function_single` and `smp::call_function_many`. Callers must be task context with interrupts enabled and cannot target their current CPU. Callbacks run in hardirq context and must not sleep, allocate, yield, or issue another synchronous call.
