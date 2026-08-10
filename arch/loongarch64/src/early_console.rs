/// 启动阶段输出一个字节。
///
/// 当前实现由所选择的平台提供。
pub fn write_byte(byte: u8) {
    crate::platform::write_console_byte(byte);
}

/// 平台控制台输入轮询:返回一个已就绪的 RX 字节,无数据则 `None`。
///
/// LS2K1000 读取 NS16550 LSR bit 0;qemu_virt 无输入路径,恒返回 `None`。
pub fn try_read_byte() -> Option<u8> {
    crate::platform::try_read_console_byte()
}
