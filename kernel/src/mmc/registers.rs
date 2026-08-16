//! DesignWare Mobile Storage Host Controller 寄存器布局与位定义。
//!
//! 参考 DW_mobile_storage_host_databook（DW-MMC）与 Linux `dw_mmc` 驱动。
//! 所有访问经 [`super::dw_mmc::MmcRegisterIo`] 抽象，真机为 volatile MMIO。

/// 寄存器偏移。
pub const REG_CTRL: usize = 0x000;
pub const REG_PWREN: usize = 0x004;
pub const REG_CLKDIV: usize = 0x008;
pub const REG_CLKSRC: usize = 0x00c;
pub const REG_CLKENA: usize = 0x010;
pub const REG_TMOUT: usize = 0x014;
pub const REG_CTYPE: usize = 0x018;
pub const REG_BLKSIZ: usize = 0x01c;
pub const REG_BYTCNT: usize = 0x020;
pub const REG_INTMASK: usize = 0x024;
pub const REG_CMDARG: usize = 0x028;
pub const REG_CMD: usize = 0x02c;
pub const REG_RESP0: usize = 0x030;
pub const REG_RESP1: usize = 0x034;
pub const REG_RESP2: usize = 0x038;
pub const REG_RESP3: usize = 0x03c;
pub const REG_MINTSTS: usize = 0x040;
pub const REG_RINTSTS: usize = 0x044;
pub const REG_STATUS: usize = 0x048;
pub const REG_FIFOTH: usize = 0x04c;
pub const REG_CDETECT: usize = 0x050;
pub const REG_WRTPRT: usize = 0x054;
pub const REG_FIFO: usize = 0x060;
pub const REG_RST_N: usize = 0x078;

/// `CTRL` 位。
pub const CTRL_ABORT_READ_DATA: u32 = 1 << 7;
pub const CTRL_USE_IDMAC: u32 = 1 << 2;

/// `RST_N` 复位位（写 1 触发，硬件自清 0）。
pub const RST_CTRL: u32 = 1 << 0;
pub const RST_FIFO: u32 = 1 << 1;
pub const RST_DMA: u32 = 1 << 2;

/// `CMD` 位。
pub const CMD_INDEX_MASK: u32 = 0x3f;
pub const CMD_RESP_EXPECT: u32 = 1 << 6;
pub const CMD_RESP_LENGTH_MASK: u32 = 0x3 << 7;
pub const CMD_RESP_NONE: u32 = 0;
pub const CMD_RESP_136: u32 = 0x1 << 7;
pub const CMD_RESP_48: u32 = 0x2 << 7;
pub const CMD_CRC_CHECK: u32 = 1 << 10;
pub const CMD_INDEX_CHECK: u32 = 1 << 11;
pub const CMD_DATA_PRESENT: u32 = 1 << 12;
pub const CMD_READ: u32 = 1 << 13;
pub const CMD_SEND_INIT: u32 = 1 << 17;
pub const CMD_UPDATE_CLOCK: u32 = 1 << 18;

/// `CLKENA` 位。
pub const CLKENA_ENABLE: u32 = 1 << 0;
pub const CLKENA_DISABLE: u32 = 1 << 1;

/// `RINTSTS` / `MINTSTS` 中断位。
pub const INT_CARD_DETECT: u32 = 1 << 0;
pub const INT_REPLAY: u32 = 1 << 1;
pub const INT_CMD_DONE: u32 = 1 << 2;
pub const INT_DATA_OVER: u32 = 1 << 3;
pub const INT_TX_DATA_REQ: u32 = 1 << 4;
pub const INT_RX_DATA_REQ: u32 = 1 << 5;
pub const INT_RESP_CRC: u32 = 1 << 6;
pub const INT_DATA_CRC: u32 = 1 << 7;
pub const INT_RESP_TIMEOUT: u32 = 1 << 8;
pub const INT_DATA_TIMEOUT: u32 = 1 << 9;
pub const INT_HOST_TIMEOUT: u32 = 1 << 10;
pub const INT_FIFO_UNDERRUN: u32 = 1 << 11;
pub const INT_FIFO_OVERRUN: u32 = 1 << 12;
pub const INT_START_BIT: u32 = 1 << 13;
pub const INT_AUTO_CMD_DONE: u32 = 1 << 14;
pub const INT_END_BIT: u32 = 1 << 15;

/// 所有与命令/数据完成或错误相关的位（复位后应清空）。
pub const INT_CMD_AND_DATA: u32 = INT_CMD_DONE
    | INT_DATA_OVER
    | INT_RESP_CRC
    | INT_DATA_CRC
    | INT_RESP_TIMEOUT
    | INT_DATA_TIMEOUT
    | INT_FIFO_UNDERRUN
    | INT_FIFO_OVERRUN
    | INT_START_BIT
    | INT_END_BIT;

/// `STATUS` 位。
pub const STATUS_DATA_BUSY: u32 = 1 << 3;
pub const STATUS_DATA_STATE: u32 = 1 << 4;
pub const STATUS_RESP_BUSY: u32 = 1 << 5;
pub const STATUS_FIFO_EMPTY: u32 = 1 << 2;
pub const STATUS_FIFO_FULL: u32 = 1 << 1;
pub const STATUS_FIFO_COUNT_MASK: u32 = 0xffff << 16;

/// CTYPE 总线宽度编码：1/4/8-bit → 0/1/3。
pub fn ctype_for_width(bus_width: u8) -> u32 {
    match bus_width {
        8 => 3,
        4 => 1,
        _ => 0,
    }
}
