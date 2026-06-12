use crate::SlabError;

#[derive(Debug)]
pub enum HeapError<E> {
    Provider(E),

    Slab(SlabError<E>),

    ZeroSizedLayout,

    AllocationTooLarge,

    AddressOverflow,

    CorruptLargeAllocation,

    LayoutMismatch,

    CounterOverflow,
}

impl<E> From<SlabError<E>> for HeapError<E> {
    fn from(error: SlabError<E>) -> Self {
        Self::Slab(error)
    }
}
