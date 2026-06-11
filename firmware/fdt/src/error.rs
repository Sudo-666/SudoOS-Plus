use core::fmt;

/// FDT blob 验证错误。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FdtError {
    HeaderTooSmall,

    InvalidMagic {
        found: u32,
    },

    TotalSizeTooSmall {
        size: usize,
    },

    TotalSizeTooLarge {
        size: usize,
    },

    Truncated {
        declared: usize,
        available: usize,
    },

    InvalidStructureRange,
    InvalidStringsRange,
    InvalidReservationMap,
    AddressOverflow,

    /// 底层 FDT 解析器拒绝了这个 blob。
    ParserRejected,

    TruncatedReservationMap,
    UnterminatedReservationMap,

    InvalidReservedMemoryLayout,

    UnsupportedCellCount {
        address_cells: u32,
        size_cells: u32,
    },

    InvalidRegLength,

    DynamicReservedMemoryUnsupported,
}

impl fmt::Display for FdtError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HeaderTooSmall => {
                write!(formatter, "FDT header is truncated")
            }

            Self::InvalidMagic { found } => {
                write!(formatter, "invalid FDT magic: {found:#010x}",)
            }

            Self::TotalSizeTooSmall { size } => {
                write!(formatter, "FDT total size is too small: {size}",)
            }

            Self::TotalSizeTooLarge { size } => {
                write!(formatter, "FDT total size is unreasonably large: {size}",)
            }

            Self::Truncated {
                declared,
                available,
            } => {
                write!(
                    formatter,
                    "FDT declares {declared} bytes, \
                     but only {available} are available",
                )
            }

            Self::InvalidStructureRange => {
                write!(formatter, "FDT structure block is out of range",)
            }

            Self::InvalidStringsRange => {
                write!(formatter, "FDT strings block is out of range",)
            }

            Self::InvalidReservationMap => {
                write!(formatter, "FDT reservation map is invalid",)
            }

            Self::AddressOverflow => {
                write!(formatter, "FDT address range overflows usize",)
            }

            Self::ParserRejected => {
                write!(formatter, "FDT semantic parser rejected the blob")
            }

            Self::TruncatedReservationMap => {
                write!(formatter, "FDT memory reservation map is truncated")
            }

            Self::UnterminatedReservationMap => {
                write!(formatter, "FDT memory reservation map has no terminator",)
            }

            Self::InvalidReservedMemoryLayout => {
                write!(formatter, "invalid /reserved-memory node layout",)
            }

            Self::UnsupportedCellCount {
                address_cells,
                size_cells,
            } => {
                write!(
                    formatter,
                    "unsupported FDT cell count: \
         address-cells={address_cells}, \
         size-cells={size_cells}",
                )
            }

            Self::InvalidRegLength => {
                write!(formatter, "FDT reg property has an invalid length",)
            }

            Self::DynamicReservedMemoryUnsupported => {
                write!(
                    formatter,
                    "dynamic /reserved-memory allocation is not supported yet",
                )
            }
        }
    }
}
