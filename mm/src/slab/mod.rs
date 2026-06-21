mod allocator;
mod cache;
mod provider;
mod size_class;
#[allow(clippy::module_inception)]
mod slab;

pub use allocator::SlabAllocator;

pub use cache::SlabCacheStats;

pub use provider::PageProvider;

pub use size_class::{MAX_SLAB_OBJECT_SIZE, MIN_SLAB_OBJECT_SIZE, SIZE_CLASS_COUNT, SizeClass};

pub use slab::SlabError;
