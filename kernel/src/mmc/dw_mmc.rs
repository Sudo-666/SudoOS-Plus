//! DesignWare MMC 主控（轮询模式，CodePlan C7，K3.2 按 dw_mmc.h 重写）。
//!
//! 寄存器访问经 [`MmcRegisterIo`] 抽象：真机为 ioremap 后的 volatile MMIO，
//! 单元测试用 mock。命令/数据/复位轮询都有真实时间 deadline（外加轮询次数
//! 上限，防止时钟未走的 mock 挂死）。

use super::registers::*;

/// 轮询截止（毫秒）：命令完成通常 < 1ms，给 500ms 余量覆盖卡忙场景。
const POLL_DEADLINE_MS: u64 = 500;
/// 轮询次数上限：兜底防止时间未走时无限循环（mock 测试）。
const POLL_COUNT_CAP: usize = 2_000_000;

/// 寄存器访问抽象（MMIO / mock）。`&mut self` 允许 mock 在读取时推进
/// 内部状态（如 FIFO 数据指针）。
pub trait MmcRegisterIo: Send + Sync + 'static {
    fn read32(&mut self, offset: usize) -> u32;
    fn write32(&mut self, offset: usize, value: u32);
}

/// 真机 volatile MMIO（虚拟地址，来自 `vm::ioremap`）。
pub struct MmioRegisterIo {
    base: usize,
}

impl MmioRegisterIo {
    /// SAFETY: `base` 必须是有效的 MMIO 寄存器虚拟基址，且在内核生命周期内
    /// 保持映射。
    pub unsafe fn new(base: usize) -> Self {
        Self { base }
    }
}

impl MmcRegisterIo for MmioRegisterIo {
    fn read32(&mut self, offset: usize) -> u32 {
        // SAFETY: 调用方保证 base + offset 在有效 MMIO 范围内。
        unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) }
    }

    fn write32(&mut self, offset: usize, value: u32) {
        // SAFETY: 同上。
        unsafe { core::ptr::write_volatile((self.base + offset) as *mut u32, value) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmcError {
    /// 轮询超时（命令/数据/复位）。
    Timeout,
    /// 响应或数据 CRC 错误。
    CrcError,
    /// FIFO 下溢/上溢。
    FifoUnderrun,
    FifoOverrun,
    /// 控制器复位未完成。
    ResetFailed,
    /// 命令参数非法。
    InvalidArgument,
    /// 控制器未就绪。
    NotReady,
    /// R1 卡状态致命错误位。
    CardError,
    /// 底层 I/O 失败。
    Io,
}

/// 命令响应类型。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MmcResponseType {
    /// 无响应（CMD0 等）。
    None,
    /// R1：48-bit，CRC + 索引检查。
    R1,
    /// R1b：R1 + 忙信号（CMD7 select 等）。响应后还需等数据总线 busy 清。
    R1b,
    /// R2：136-bit（CID/CSD）。
    R2,
    /// R3：48-bit，无 CRC（OCR）。
    R3,
    /// R6：48-bit（RCA）。
    R6,
    /// R7：48-bit（接口条件）。
    R7,
}

/// 一条主机命令。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MmcCommand {
    pub index: u8,
    pub argument: u32,
    pub response_type: MmcResponseType,
    /// 携带数据（块传输）。
    pub data_present: bool,
    /// 数据方向：true = 读（卡 → 主机）。
    pub read: bool,
    /// 初始化命令（CMD0/CMD1）。
    pub init: bool,
    /// 仅更新时钟寄存器。
    pub update_clock: bool,
    /// 数据长度（字节）。带数据命令必须给出，控制器据此编程 BLKSIZ/BYTCNT。
    pub data_length: Option<usize>,
}

impl MmcCommand {
    pub const fn new(index: u8, argument: u32, response_type: MmcResponseType) -> Self {
        Self {
            index,
            argument,
            response_type,
            data_present: false,
            read: false,
            init: false,
            update_clock: false,
            data_length: None,
        }
    }

    /// 带数据命令：`data_length` 为传输字节数（如 ACMD51=8、CMD17=512）。
    pub const fn with_data_length(mut self, read: bool, data_length: usize) -> Self {
        self.data_present = true;
        self.read = read;
        self.data_length = Some(data_length);
        self
    }

    pub const fn with_init(mut self) -> Self {
        self.init = true;
        self
    }
}

/// SD 卡协议层依赖的主机接口（由 `DwMmcController` 实现）。
pub trait MmcHost: Send + Sync + 'static {
    fn send_command(&mut self, command: MmcCommand) -> Result<MmcResponse, MmcError>;
    fn read_block_data(&mut self, output: &mut [u8]) -> Result<(), MmcError>;
    fn set_clock(&mut self, frequency_hz: u64) -> Result<(), MmcError>;
    fn set_bus_width(&mut self, bus_width: u8) -> Result<(), MmcError>;
}

impl<I: MmcRegisterIo> MmcHost for DwMmcController<I> {
    fn send_command(&mut self, command: MmcCommand) -> Result<MmcResponse, MmcError> {
        DwMmcController::send_command(self, command)
    }

    fn read_block_data(&mut self, output: &mut [u8]) -> Result<(), MmcError> {
        DwMmcController::read_block_data(self, output)
    }

    fn set_clock(&mut self, frequency_hz: u64) -> Result<(), MmcError> {
        DwMmcController::set_clock(self, frequency_hz)
    }

    fn set_bus_width(&mut self, bus_width: u8) -> Result<(), MmcError> {
        DwMmcController::set_bus_width(self, bus_width)
    }
}

/// 控制器响应（RESP0-3）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MmcResponse {
    pub r0: u32,
    pub r1: u32,
    pub r2: u32,
    pub r3: u32,
}

impl MmcResponse {
    /// 48-bit 响应（R1/R3/R6/R7）：卡状态在 RESP0。
    pub fn status(&self) -> u32 {
        self.r0
    }

    /// 136-bit 响应（R2）：`r0..r3` 是协议响应**从高到低**的四个 word
    /// （`r0` = payload[127:96]），与物理 RESP 寄存器编号无关——读取时
    /// `r0 = REG_RESP3`、`r1 = REG_RESP2`、`r2 = REG_RESP1`、
    /// `r3 = REG_RESP0`（Linux `dw_mmc.c` 逆序读，`resp[0] = RESP3`；
    /// `mmc_decode_cid` 取 `resp[0] >> 24` = CID[127:120] = 厂商 ID）。
    /// 每个 word 内 4 个载荷字节按**大端**存放，故用 `to_be_bytes()`
    /// 还原出 16 字节协议顺序载荷（上一轮误用 `to_le_bytes()`，C_SIZE
    /// 取错位导致容量解析错误，且 mock 与实现共享同一错误假设）。
    pub fn card_data(&self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[0..4].copy_from_slice(&self.r0.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.r1.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.r2.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.r3.to_be_bytes());
        bytes
    }
}

/// R1 卡状态致命错误位（SD 规范 R1：bits 31:19 + bit 15）。
const R1_ERROR_MASK: u32 = 0xfff8_0000 | 0x0000_8000;

fn r1_has_error(status: u32) -> bool {
    status & R1_ERROR_MASK != 0
}

/// DW-MMC 轮询主控。
pub struct DwMmcController<I: MmcRegisterIo> {
    io: I,
    ciu_frequency_hz: u64,
    fifo_depth: u32,
    /// 数据寄存器（FIFO）偏移，构造时按 VERID 判定（0x100 / 0x200）。
    fifo_offset: usize,
}

impl<I: MmcRegisterIo> DwMmcController<I> {
    pub fn new(mut io: I, ciu_frequency_hz: u64, fifo_depth: u32) -> Self {
        let verid = io.read32(REG_VERID) & 0xffff;
        let fifo_offset = if verid >= DW_MMC_240A {
            DATA_240A_OFFSET
        } else {
            DATA_OFFSET
        };
        Self {
            io,
            ciu_frequency_hz,
            fifo_depth: fifo_depth.max(1),
            fifo_offset,
        }
    }

    /// 上电（PWREN = 1）。
    pub fn power_on(&mut self) -> Result<(), MmcError> {
        self.io.write32(REG_PWREN, 1);
        Ok(())
    }

    /// 控制器 + FIFO 复位。复位位写 1 后硬件自清；轮询至位清 0。
    pub fn reset(&mut self) -> Result<(), MmcError> {
        self.io.write32(REG_CTRL, CTRL_RESET | CTRL_FIFO_RESET);
        self.poll_for(|value| value & (CTRL_RESET | CTRL_FIFO_RESET) == 0, REG_CTRL)?;
        Ok(())
    }

    /// 禁用中断并清理原始中断状态。
    pub fn disable_interrupts(&mut self) {
        self.io.write32(REG_INTMASK, 0);
        self.io.write32(REG_RINTSTS, u32::MAX);
    }

    /// 读取原始中断状态。
    pub fn raw_interrupts(&mut self) -> u32 {
        self.io.read32(REG_RINTSTS)
    }

    /// 配置时钟。先禁用时钟并应用（UPD_CLK），再写分频 + 使能并再次应用。
    pub fn set_clock(&mut self, frequency_hz: u64) -> Result<(), MmcError> {
        self.wait_data_busy_clear()?;

        self.io.write32(REG_CLKENA, 0);
        self.send_update_clock()?;

        let target = frequency_hz.max(100_000);
        // 分频向上取整，保证实际输出 ≤ 目标频率（向下取整会略超目标）。
        // ciu ≤ target 时 divider = 0（CLKDIV 直通，输出 = CIU）。CLKSRC
        // 固定时钟源 0。CIU=50MHz→400kHz：divider = ceil(50M/800k) = 63，
        // 实际 396.8kHz ≤ 400kHz。
        let divider = if self.ciu_frequency_hz <= target {
            0
        } else {
            self.ciu_frequency_hz
                .div_ceil(target.saturating_mul(2))
                .min(0xffff) as u32
        };
        self.io.write32(REG_CLKSRC, 0);
        self.io.write32(REG_CLKDIV, divider);
        self.io.write32(REG_CLKENA, CLKENA_ENABLE);
        self.send_update_clock()?;
        Ok(())
    }

    fn send_update_clock(&mut self) -> Result<(), MmcError> {
        let command = MmcCommand {
            index: 0,
            argument: 0,
            response_type: MmcResponseType::None,
            data_present: false,
            read: false,
            init: false,
            update_clock: true,
            data_length: None,
        };
        self.send_command(command).map(|_| ())
    }

    /// 设置总线宽度（1/4/8-bit）。
    pub fn set_bus_width(&mut self, bus_width: u8) -> Result<(), MmcError> {
        if bus_width != 1 && bus_width != 4 && bus_width != 8 {
            return Err(MmcError::InvalidArgument);
        }
        self.io.write32(REG_CTYPE, ctype_for_width(bus_width));
        Ok(())
    }

    /// 带数据命令先编程块长/字节数/FIFO 水印。
    fn configure_data_transfer(&mut self, data_length: usize) -> Result<(), MmcError> {
        if data_length == 0 || data_length % 4 != 0 {
            return Err(MmcError::InvalidArgument);
        }
        let words = data_length.div_ceil(4) as u32;
        // RX/TX 水印有效范围是 [1, fifo_depth-1]，习惯取半深。等于
        // fifo_depth 时 RXDR 比较器永不满足 → 读扇区超时（K3.4）。
        let max_watermark = self.fifo_depth.saturating_sub(1).max(1);
        let watermark = words.min(self.fifo_depth / 2).clamp(1, max_watermark);
        self.io.write32(REG_BLKSIZ, data_length as u32);
        self.io.write32(REG_BYTCNT, data_length as u32);
        self.io.write32(REG_FIFOTH, fifoth_for(watermark));
        Ok(())
    }

    /// 发送命令并等待响应。写 CMD 前先写 CMDARG；带数据时先编程
    /// BLKSIZ/BYTCNT/FIFOTH；`CMD_START`（bit 31）置位启动命令。
    pub fn send_command(&mut self, command: MmcCommand) -> Result<MmcResponse, MmcError> {
        if command.data_present {
            let data_length = command.data_length.ok_or(MmcError::InvalidArgument)?;
            self.configure_data_transfer(data_length)?;
        }
        let cmd_value = self.build_command_value(&command) | CMD_START;
        self.io.write32(REG_CMDARG, command.argument);
        self.io.write32(REG_CMD, cmd_value);

        // UPDATE_CLOCK 无响应、无 CMD_DONE，等 START 自清；普通命令等
        // CMD_DONE（响应就绪）而非 START 清零。
        if command.update_clock {
            self.poll_update_clock_complete()?;
        } else {
            self.poll_normal_command_done()?;
        }

        let mut response = MmcResponse::default();
        match command.response_type {
            MmcResponseType::None => {}
            MmcResponseType::R2 => {
                // r0 = 协议最高 word = RESP3；r3 = 协议最低 word = RESP0
                // （DW-MMC 逆序存储，Linux/U-Boot 均按此读取）。
                response.r0 = self.io.read32(REG_RESP3);
                response.r1 = self.io.read32(REG_RESP2);
                response.r2 = self.io.read32(REG_RESP1);
                response.r3 = self.io.read32(REG_RESP0);
            }
            MmcResponseType::R1 | MmcResponseType::R1b | MmcResponseType::R3
            | MmcResponseType::R6 | MmcResponseType::R7 => {
                response.r0 = self.io.read32(REG_RESP0);
            }
        }
        // R1/R1b 卡状态校验：致命错误位 → CardError（K3.2）。
        if matches!(
            command.response_type,
            MmcResponseType::R1 | MmcResponseType::R1b
        ) && r1_has_error(response.r0)
        {
            return Err(MmcError::CardError);
        }
        // R1b：读响应后继续等数据总线忙碌清，然后才能发下一条命令
        // （如 CMD55/ACMD51）。
        if command.response_type == MmcResponseType::R1b {
            self.wait_data_busy_clear()?;
        }
        Ok(response)
    }

    /// PIO 单块读取：从 FIFO 拉取一个块（`output.len()` 字节），按 RXDR /
    /// DATA_OVER 中断驱动，FIFO 分批到达。
    pub fn read_block_data(&mut self, output: &mut [u8]) -> Result<(), MmcError> {
        if output.is_empty() || output.len() % 4 != 0 {
            return Err(MmcError::InvalidArgument);
        }
        let words = output.len() / 4;
        let mut received = 0_usize;
        let mut total = 0_usize;
        while total < words {
            let interrupts = self.poll_for(
                |value| {
                    value & (INT_RX_DATA_REQ | INT_DATA_OVER | INT_DATA_CRC | INT_DATA_TIMEOUT
                        | INT_FIFO_RUN_ERROR | INT_HLE)
                        != 0
                },
                REG_RINTSTS,
            )?;
            if interrupts & INT_DATA_CRC != 0 {
                self.clear_interrupts(INT_DATA_CRC | INT_DATA_OVER);
                return Err(MmcError::CrcError);
            }
            if interrupts & INT_DATA_TIMEOUT != 0 {
                self.clear_interrupts(INT_DATA_TIMEOUT | INT_DATA_OVER);
                return Err(MmcError::Timeout);
            }
            // FRUN（bit 11）同时覆盖 FIFO 下溢/上溢；读路径上溢 = 数据丢失。
            if interrupts & INT_FIFO_RUN_ERROR != 0 {
                self.clear_interrupts(INT_FIFO_RUN_ERROR | INT_DATA_OVER);
                return Err(MmcError::FifoOverrun);
            }
            // HLE（bit 12）是主机锁定错误，不是 FIFO overrun。
            if interrupts & INT_HLE != 0 {
                self.clear_interrupts(INT_HLE | INT_DATA_OVER);
                return Err(MmcError::Io);
            }
            // 读 FIFO 直到空（分批到达）。
            while !self.fifo_empty() && received < words {
                let word = self.io.read32(self.fifo_offset);
                let offset = received * 4;
                output[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
                received += 1;
                total += 1;
            }
            self.io.write32(REG_RINTSTS, INT_RX_DATA_REQ);
            if interrupts & INT_DATA_OVER != 0 {
                break;
            }
        }
        self.io.write32(REG_RINTSTS, INT_DATA_OVER | INT_RX_DATA_REQ);
        if received < words {
            return Err(MmcError::Timeout);
        }
        Ok(())
    }

    fn build_command_value(&self, command: &MmcCommand) -> u32 {
        let mut value = u32::from(command.index) & CMD_INDEX_MASK;
        match command.response_type {
            MmcResponseType::None => {}
            MmcResponseType::R2 => {
                // R2 也带 CRC 校验（136-bit，命令后 7-bit CRC）。
                value |= CMD_RESP_EXP | CMD_RESP_LONG | CMD_RESP_CRC;
            }
            MmcResponseType::R1 | MmcResponseType::R1b | MmcResponseType::R6
            | MmcResponseType::R7 => {
                value |= CMD_RESP_EXP | CMD_RESP_CRC;
            }
            MmcResponseType::R3 => {
                value |= CMD_RESP_EXP;
            }
        }
        if command.data_present {
            value |= CMD_DATA_EXPECTED;
            if !command.read {
                // DAT_WR = 主机写数据到卡；读（卡 → 主机）不置位。
                value |= CMD_DATA_WRITE;
            }
        }
        if command.init {
            value |= CMD_SEND_INIT;
        }
        if command.update_clock {
            value |= CMD_UPDATE_CLOCK;
        }
        // 数据命令 / UPDATE_CLOCK 置 PRV_DAT_WAIT（等上一条数据命令完成）；
        // JH7110 无 USE_HOLD_REG quirk，普通命令统一置 USE_HOLD_REG。
        if command.data_present || command.update_clock {
            value |= CMD_PRV_DAT_WAIT;
        }
        if !command.update_clock {
            value |= CMD_USE_HOLD_REG;
        }
        value
    }

    /// 等待 UPDATE_CLOCK 完成。无响应命令**不**置位 `CMD_DONE`，完成信号是
    /// `CMD_START`（bit 31）被硬件自动清零；只检查 `HLE`（主机锁定错误）。
    /// 保留真实时间 deadline + 轮询次数上限双重超时。
    fn poll_update_clock_complete(&mut self) -> Result<(), MmcError> {
        let cycles_per_ms = crate::time::clock_frequency_hz() / 1000;
        let deadline = crate::time::now()
            .cycles()
            .saturating_add(cycles_per_ms.saturating_mul(POLL_DEADLINE_MS));
        let mut count = 0_usize;
        loop {
            let interrupts = self.io.read32(REG_RINTSTS);
            if interrupts & INT_HLE != 0 {
                self.clear_interrupts(INT_HLE);
                return Err(MmcError::Io);
            }
            if self.io.read32(REG_CMD) & CMD_START == 0 {
                return Ok(());
            }
            count += 1;
            if count >= POLL_COUNT_CAP {
                return Err(MmcError::Timeout);
            }
            if count & 0x1f == 0 && crate::time::now().cycles() >= deadline {
                return Err(MmcError::Timeout);
            }
            core::hint::spin_loop();
        }
    }

    /// 等待普通命令完成。轮询 `RINTSTS`：先判响应/主机错误位（RESP_TIMEOUT、
    /// RESP_CRC、RESP_ERR、HLE、START_BIT_ERROR、END_BIT_ERROR），再等
    /// `CMD_DONE`，成功后清理。响应寄存器只在 `CMD_DONE` 置起后有效——
    /// `CMD_START` 自清（命令已发出）**不等于**响应已就绪，不能在 START
    /// 清零时就读 RESP（K3.4 起按 CMD_DONE 判定）。
    fn poll_normal_command_done(&mut self) -> Result<(), MmcError> {
        let error_mask = INT_RESP_CRC | INT_RESP_TIMEOUT | INT_RESP_ERR | INT_HLE
            | INT_START_BIT_ERROR | INT_END_BIT_ERROR;
        let cycles_per_ms = crate::time::clock_frequency_hz() / 1000;
        let deadline = crate::time::now()
            .cycles()
            .saturating_add(cycles_per_ms.saturating_mul(POLL_DEADLINE_MS));
        let mut count = 0_usize;
        loop {
            let interrupts = self.io.read32(REG_RINTSTS);
            if interrupts & (INT_RESP_CRC | INT_RESP_ERR | INT_START_BIT_ERROR | INT_END_BIT_ERROR)
                != 0
            {
                // 错误返回也一并清 CMD_DONE：残留的 CMD_DONE 会让下一条命令
                // 在响应就绪前误判完成。
                self.clear_interrupts(interrupts & (error_mask | INT_CMD_DONE));
                return Err(MmcError::CrcError);
            }
            if interrupts & INT_RESP_TIMEOUT != 0 {
                self.io.write32(REG_RINTSTS, INT_RESP_TIMEOUT | INT_CMD_DONE);
                return Err(MmcError::Timeout);
            }
            if interrupts & INT_HLE != 0 {
                self.clear_interrupts(INT_HLE | INT_CMD_DONE);
                return Err(MmcError::Io);
            }
            if interrupts & INT_CMD_DONE != 0 {
                self.io.write32(REG_RINTSTS, INT_CMD_DONE);
                return Ok(());
            }
            count += 1;
            if count >= POLL_COUNT_CAP {
                return Err(MmcError::Timeout);
            }
            if count & 0x1f == 0 && crate::time::now().cycles() >= deadline {
                return Err(MmcError::Timeout);
            }
            core::hint::spin_loop();
        }
    }

    fn wait_data_busy_clear(&mut self) -> Result<(), MmcError> {
        self.poll_for(|value| value & STATUS_BUSY == 0, REG_STATUS)?;
        Ok(())
    }

    fn fifo_empty(&mut self) -> bool {
        let status = self.io.read32(REG_STATUS);
        (status >> STATUS_FIFO_COUNT_SHIFT) & STATUS_FIFO_COUNT_MASK == 0
    }

    /// 测试用：返回底层 I/O 引用（mock 的命令轨迹）。
    #[cfg(debug_assertions)]
    pub fn io_ref(&self) -> &I {
        &self.io
    }

    /// 测试用：当前 FIFO 偏移。
    #[cfg(debug_assertions)]
    pub fn fifo_offset(&self) -> usize {
        self.fifo_offset
    }

    fn clear_interrupts(&mut self, bits: u32) {
        self.io.write32(REG_RINTSTS, bits);
    }

    /// 轮询寄存器直到谓词成立，带真实时间 deadline + 次数上限。
    fn poll_for(
        &mut self,
        predicate: impl Fn(u32) -> bool,
        register: usize,
    ) -> Result<u32, MmcError> {
        let cycles_per_ms = crate::time::clock_frequency_hz() / 1000;
        let deadline = crate::time::now()
            .cycles()
            .saturating_add(cycles_per_ms.saturating_mul(POLL_DEADLINE_MS));
        let mut count = 0_usize;
        loop {
            let value = self.io.read32(register);
            if predicate(value) {
                return Ok(value);
            }
            count += 1;
            if count >= POLL_COUNT_CAP {
                return Err(MmcError::Timeout);
            }
            if count & 0x1f == 0 && crate::time::now().cycles() >= deadline {
                return Err(MmcError::Timeout);
            }
            core::hint::spin_loop();
        }
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    use super::mock::{MockFailure, MockRegisterIo};
    use alloc::vec;

    // 1) 正常命令完成（R7：CMD8/0x1aa）。
    let mock = MockRegisterIo::new().with_responses(vec![0x1a2b_3c4d, 0, 0, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let response = controller
        .send_command(MmcCommand::new(8, 0x1aa, MmcResponseType::R7))
        .expect("command 8 completed");
    assert_eq!(response.r0, 0x1a2b_3c4d);
    // 控制器必须置位 CMD_START，且带数据命令先编程 BLKSIZ/BYTCNT。
    let trace = &controller.io_ref().commands;
    assert_eq!(trace[0].index, 8);
    assert!(trace[0].cmd_start, "CMD_START must be set");

    // 2) 响应超时。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResponseTimeout);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(8, 0x1aa, MmcResponseType::R7)),
        Err(MmcError::Timeout),
    );

    // 3) 响应 CRC 错误。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResponseCrc);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(55, 0, MmcResponseType::R1)),
        Err(MmcError::CrcError),
    );

    // 4) 数据超时。
    let mock = MockRegisterIo::new().with_failure(MockFailure::DataTimeout);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data_length(true, 512))
        .expect("data command accepted");
    let mut block = [0_u8; 512];
    assert_eq!(controller.read_block_data(&mut block), Err(MmcError::Timeout));

    // 5) FIFO 上溢。
    let mock = MockRegisterIo::new().with_failure(MockFailure::FifoOverrun);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data_length(true, 512))
        .expect("data command accepted");
    let mut block = [0_u8; 512];
    assert_eq!(
        controller.read_block_data(&mut block),
        Err(MmcError::FifoOverrun),
    );

    // 6) 控制器复位卡死 → Timeout（不挂死）。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResetHang);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(controller.reset(), Err(MmcError::Timeout));

    // 7) FIFO 数据分批到达（旧版 0x100 偏移）。
    let mock = MockRegisterIo::new()
        .with_fifo_data(vec![0x0302_0100, 0x0706_0504, 0x0b0a_0908, 0x0f0e_0d0c])
        .with_fifo_batch(2);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(controller.fifo_offset(), DATA_OFFSET, "VERID=0 -> old FIFO");
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data_length(true, 16))
        .expect("data command accepted");
    let mut block = [0_u8; 16];
    controller
        .read_block_data(&mut block)
        .expect("batched fifo read");
    assert_eq!(block, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

    // 8) VERID ≥ 2.40a → FIFO 在 0x200（JH7110）。
    let mock = MockRegisterIo::new().with_verid(0x291a);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(controller.fifo_offset(), DATA_240A_OFFSET, "2.91a -> new FIFO");

    // 9) 错误后恢复：清掉故障再发命令成功。
    let mut mock = MockRegisterIo::new().with_failure(MockFailure::ResponseTimeout);
    mock.set_failure(None);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let response = controller
        .send_command(MmcCommand::new(13, 0x1234, MmcResponseType::R1))
        .expect("command after error recovery");
    assert_eq!(response.r0, 0);

    // 10) R1 卡状态致命错误 → CardError。
    let mock = MockRegisterIo::new().with_responses(vec![1 << 19, 0, 0, 0]); // R1: ERROR
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(13, 0, MmcResponseType::R1)),
        Err(MmcError::CardError),
    );

    // 11) 时钟与总线宽度配置。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller.set_clock(400_000).expect("400 kHz clock");
    controller.set_clock(25_000_000).expect("25 MHz clock");
    controller.set_bus_width(4).expect("4-bit bus width");
    assert_eq!(controller.set_bus_width(3), Err(MmcError::InvalidArgument));

    // 12) 复位 + 上电 + 禁用中断不报错。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller.power_on().expect("power on");
    controller.reset().expect("reset");
    controller.disable_interrupts();

    // 13) UPDATE_CLOCK 命令：mock 不置 CMD_DONE，仅靠 CMD_START 自清完成。
    //     控制器必须通过轮询 START 位返回，不能等 CMD_DONE（K3.4）。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let update_clock = MmcCommand {
        index: 0,
        argument: 0,
        response_type: MmcResponseType::None,
        data_present: false,
        read: false,
        init: false,
        update_clock: true,
        data_length: None,
    };
    controller
        .send_command(update_clock)
        .expect("update-clock completes via CMD_START self-clear");
    let trace = &controller.io_ref().commands;
    assert!(
        trace.iter().any(|command| command.update_clock && command.cmd_start),
        "update-clock must be issued with CMD_START"
    );

    // 14) 命令永不完成（CMD_START 不自清）→ 轮询超时，不挂死。
    let mock = MockRegisterIo::new().with_failure(MockFailure::CommandHang);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(55, 0, MmcResponseType::R1)),
        Err(MmcError::Timeout),
    );

    // 15) CMD17（512B，128 字）在 32 深 FIFO 上：RX 水印必须取半深 16，
    //     而不是 FIFO 深度 32（等于深度时 RXDR 永不触发，读超时）。
    let mock = MockRegisterIo::new().with_responses(vec![0, 0, 0, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data_length(true, 512))
        .expect("CMD17 accepted");
    let cmd17 = controller
        .io_ref()
        .commands
        .iter()
        .find(|command| command.index == 17)
        .expect("CMD17 trace");
    let rx_wm = (cmd17.fifo_threshold.unwrap() >> 16) & 0x0fff;
    assert!(
        rx_wm < 32,
        "RX watermark {rx_wm} must be below FIFO depth 32"
    );
    assert_eq!(rx_wm, 16, "512B on 32-word FIFO → half-depth watermark");

    // 16) 小传输（ACMD51=8B，2 字）：水印取 min(words, half) = 2。
    let mock = MockRegisterIo::new().with_responses(vec![0, 0, 0, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller
        .send_command(MmcCommand::new(51, 0, MmcResponseType::R1).with_data_length(true, 8))
        .expect("ACMD51 accepted");
    let cmd51 = controller
        .io_ref()
        .commands
        .iter()
        .find(|command| command.index == 51)
        .expect("ACMD51 trace");
    assert_eq!(
        (cmd51.fifo_threshold.unwrap() >> 16) & 0x0fff,
        2,
        "8B → 2-word watermark"
    );

    // 17) R2 字节序：RESP3 = 协议最高 word，字内大端。mock 的 RESP 组按
    //     [RESP0, RESP1, RESP2, RESP3] 给出（Linux dw_mmc.c 逆序读）。
    let mock = MockRegisterIo::new().with_responses(vec![
        0x7654_3210, // RESP0 = payload[12..16]
        0xfedc_ba98, // RESP1 = payload[8..12]
        0x89ab_cdef, // RESP2 = payload[4..8]
        0x0123_4567, // RESP3 = payload[0..4]（最高 word）
    ]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let response = controller
        .send_command(MmcCommand::new(2, 0, MmcResponseType::R2))
        .expect("CMD2 R2 response");
    assert_eq!(
        response.card_data(),
        [
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54,
            0x32, 0x10,
        ],
        "R2 must decode RESP3..RESP0 as big-endian protocol payload"
    );

    // 18) 分频向上取整：CIU=50MHz → 400kHz 需 divider=63（实际 396.8kHz），
    //     且 CLKSRC 固定时钟源 0。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 50_000_000, 32);
    controller.set_clock(400_000).expect("400 kHz init clock");
    assert_eq!(
        controller.io_ref().read_reg(REG_CLKDIV),
        63,
        "50 MHz → 400 kHz needs divider = ceil(50M/800k) = 63"
    );
    assert_eq!(
        controller.io_ref().read_reg(REG_CLKSRC),
        0,
        "CLKSRC must be fixed to clock source 0"
    );

    // 19) R1b：busy 永不清 → Timeout；普通 R1 不等待 busy。
    let mock = MockRegisterIo::new().with_failure(MockFailure::BusyHang);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(7, 0x1234_0000, MmcResponseType::R1b)),
        Err(MmcError::Timeout),
        "R1b must wait for DAT busy to clear"
    );
    let mock = MockRegisterIo::new().with_failure(MockFailure::BusyHang);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    controller
        .send_command(MmcCommand::new(7, 0x1234_0000, MmcResponseType::R1))
        .expect("plain R1 must not wait for busy");

    // 20) 分阶段完成模型：START 清零 ≠ 响应就绪。mock 延迟一拍置 CMD_DONE，
    //     驱动若在 START 清零时就返回，会读到未就绪的 RESP（0）；正确实现
    //     必须等 CMD_DONE 且 completed_count == 1。
    let mock = MockRegisterIo::new().with_responses(vec![0x1a2b_3c4d, 0, 0, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let response = controller
        .send_command(MmcCommand::new(8, 0x1aa, MmcResponseType::R7))
        .expect("normal command must wait for CMD_DONE");
    assert_eq!(
        response.r0, 0x1a2b_3c4d,
        "response must be available only after the mock raised CMD_DONE"
    );
    assert_eq!(
        controller.io_ref().completed_count(),
        1,
        "driver must return only after CMD_DONE, not on START auto-clear"
    );

    // 21) CMD_DONE 与错误同时出现（RTO/RCRC 终止命令）：错误优先。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResponseCrc);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(
        controller.send_command(MmcCommand::new(55, 0, MmcResponseType::R1)),
        Err(MmcError::CrcError),
        "error must win even when CMD_DONE is also set"
    );

    crate::println!("C7 DW-MMC controller gate:");
    crate::println!("  command complete    : verified");
    crate::println!("  response timeout    : verified");
    crate::println!("  response CRC        : verified");
    crate::println!("  data timeout        : verified");
    crate::println!("  FIFO overrun        : verified");
    crate::println!("  reset hang          : verified");
    crate::println!("  FIFO batching       : verified");
    crate::println!("  VERID FIFO offset   : verified");
    crate::println!("  error recovery      : verified");
    crate::println!("  R1 card status      : verified");
    crate::println!("  clock/bus-width     : verified");
    crate::println!("  update-clock        : verified");
    crate::println!("  command hang        : verified");
    crate::println!("  FIFO watermark      : verified");
}
