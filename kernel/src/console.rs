use core::fmt;

use myos_runtime::console::ByteConsole;

/// 将当前架构的早期控制台适配到公共格式化设施。
struct EarlyConsole;

impl ByteConsole for EarlyConsole {
    #[inline]
    fn write_byte(byte: u8) {
        crate::arch::early_console::write_byte(byte);
    }
}

#[doc(hidden)]
pub fn print(arguments: fmt::Arguments<'_>) {
    myos_runtime::console::write::<EarlyConsole>(arguments);
}

#[macro_export]
macro_rules! print {
    ($($argument:tt)*) => {
        $crate::console::print(format_args!($($argument)*))
    };
}

#[macro_export]
macro_rules! println {
    () => {
        $crate::print!("\n")
    };

    ($($argument:tt)*) => {
        $crate::print!("{}\n", format_args!($($argument)*))
    };
}
