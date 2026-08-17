//! DesignWare Mobile Storage Host Controller 寄存器布局与位定义。
//!
//! 以 Linux `drivers/mmc/host/dw_mmc.h` 为权威源校准（K3.2/PR3）。JH7110
//! 使用新版 DW-MMC（≥ 2.40a），数据寄存器（FIFO）位于 `0x200`。旧实现的
//! CMD/STATUS/CTRL 位整体偏移、FIFO 偏移错误（`0x060` 是 TBBCNT 而非
//! FIFO），mock 与实现共享同一套错误定义，PASS 不能证明真机可用。
//!
//! 所有访问经 [`super::dw_mmc::MmcRegisterIo`] 抽象，真机为 ioremap 后的
//! volatile MMIO。

/// 寄存器偏移（dw_mmc.h）。
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
pub const REG_VERID: usize = 0x06c;
pub const REG_HCON: usize = 0x070;
pub const REG_RST_N: usize = 0x078;

/// 数据寄存器（FIFO）偏移：低于 2.40a 为 `0x100`，≥ 2.40a 为 `0x200`
/// （dw_mmc.h `DATA_OFFSET` / `DATA_240A_OFFSET`）。
pub const DATA_OFFSET: usize = 0x100;
pub const DATA_240A_OFFSET: usize = 0x200;
/// VERID 判定 FIFO 偏移的版本阈值（dw_mmc.h `DW_MMC_240A = 0x240a`）。
pub const DW_MMC_240A: u32 = 0x240a;

/// `CTRL` 位。
pub const CTRL_USE_IDMAC: u32 = 1 << 25;
pub const CTRL_ABORT_READ_DATA: u32 = 1 << 8;
pub const CTRL_DMA_RESET: u32 = 1 << 2;
pub const CTRL_FIFO_RESET: u32 = 1 << 1;
pub const CTRL_RESET: u32 = 1 << 0;

/// `CLKENA` 位（只有使能位；禁用在 UPD_CLK 前清 ENABLE）。
pub const CLKENA_ENABLE: u32 = 1 << 0;

/// `CMD` 位（新版布局）。`CMD_START`（bit 31）写 1 启动命令，硬件自清。
pub const CMD_INDEX_MASK: u32 = 0x3f;
pub const CMD_RESP_EXP: u32 = 1 << 6;
pub const CMD_RESP_LONG: u32 = 1 << 7;
pub const CMD_RESP_CRC: u32 = 1 << 8;
pub const CMD_DATA_EXPECTED: u32 = 1 << 9;
pub const CMD_DATA_WRITE: u32 = 1 << 10;
pub const CMD_SEND_INIT: u32 = 1 << 15;
pub const CMD_UPDATE_CLOCK: u32 = 1 << 21;
pub const CMD_START: u32 = 1 << 31;

/// `RINTSTS` / `MINTSTS` 中断位。
pub const INT_CARD_DETECT: u32 = 1 << 0;
pub const INT_RESP_ERR: u32 = 1 << 1;
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
/// 读路径把 HLE（bit 12，主机锁定错误）作为 FIFO/数据错误上溢上报。
pub const INT_FIFO_OVERRUN: u32 = 1 << 12;
pub const INT_START_BIT: u32 = 1 << 13;
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

/// `STATUS` 位：`DATA_BUSY` = bit 9；FIFO 计数在 bits [29:17]。
pub const STATUS_BUSY: u32 = 1 << 9;
pub const STATUS_FIFO_COUNT_SHIFT: u32 = 17;
pub const STATUS_FIFO_COUNT_MASK: u32 = 0x1fff;

/// FIFOTH 值：`m | rx<<16 | tx`（水印以数据字为单位）。
pub const fn fifoth_for(rx_watermark: u32) -> u32 {
    ((rx_watermark & 0xfff) << 16) | (rx_watermark & 0xfff)
}

/// CTYPE 总线宽度编码：1/4/8-bit → 0 / bit0 / bit16。
pub fn ctype_for_width(bus_width: u8) -> u32 {
    match bus_width {
        8 => 1 << 16,
        4 => 1,
        _ => 0,
    }
}
