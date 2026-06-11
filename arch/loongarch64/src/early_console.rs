/// 启动阶段输出一个字节。
///
/// 当前实现由所选择的平台提供。
pub fn write_byte(byte: u8) {
    crate::platform::write_console_byte(byte);
}
