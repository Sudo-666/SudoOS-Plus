use core::{
    fmt::{self, Write},
    marker::PhantomData,
};

/// 最底层的字节输出能力。
///
/// 实现者不需要保存任何状态，通常只是把一个字节写入
/// 架构或平台提供的早期串口。
pub trait ByteConsole {
    fn write_byte(byte: u8);
}

/// 将一个 [`ByteConsole`] 适配为 [`core::fmt::Write`]。
///
/// 类型参数用于在编译期选择具体控制台，不产生运行时
/// trait object 或虚函数调用。
pub struct ConsoleWriter<C> {
    marker: PhantomData<C>,
}

impl<C> ConsoleWriter<C> {
    pub const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<C> Default for ConsoleWriter<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: ByteConsole> Write for ConsoleWriter<C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            /*
             * 串口终端通常使用 CRLF。
             *
             * 上层统一写 '\n'，这里完成终端格式转换。
             */
            if byte == b'\n' {
                C::write_byte(b'\r');
            }

            C::write_byte(byte);
        }

        Ok(())
    }
}

/// 使用指定的控制台类型输出格式化参数。
///
/// 这里不返回错误，因为早期串口不存在有意义的错误恢复路径。
pub fn write<C: ByteConsole>(arguments: fmt::Arguments<'_>) {
    let mut writer = ConsoleWriter::<C>::new();

    let _ = writer.write_fmt(arguments);
}
