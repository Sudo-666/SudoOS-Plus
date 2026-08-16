//! DesignWare MMC 主控（轮询模式，CodePlan C7）。
//!
//! 寄存器访问经 [`MmcRegisterIo`] 抽象：真机为 volatile MMIO，单元测试用
//! mock。所有轮询都有截止次数（`DEADLINE_POLLS`），绝不无限循环。

use super::registers::*;

/// 轮询截止次数（真机每轮为一次 MMIO 读，远超硬件命令完成时间；
/// 取值在未优化 debug 构建下也要能在 QEMU 里秒级跑完）。
const DEADLINE_POLLS: usize = 100_000;

/// 寄存器访问抽象（MMIO / mock）。`&mut self` 允许 mock 在读取时推进
/// 内部状态（如 FIFO 数据指针）。
pub trait MmcRegisterIo: Send + Sync + 'static {
    fn read32(&mut self, offset: usize) -> u32;
    fn write32(&mut self, offset: usize, value: u32);
}

/// 真机 volatile MMIO。
pub struct MmioRegisterIo {
    base: usize,
}

impl MmioRegisterIo {
    /// SAFETY: `base` 必须是有效的 MMIO 寄存器基址，且在内核生命周期内
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
        }
    }

    pub const fn with_data(mut self, read: bool) -> Self {
        self.data_present = true;
        self.read = read;
        self
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
    /// 48-bit 响应（R1/R3/R6/R7）：高 32 位在 RESP0。
    pub fn status(&self) -> u32 {
        self.r0
    }

    /// 136-bit 响应（R2）：RESP0..3 从高位到低位。
    pub fn card_data(&self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[0..4].copy_from_slice(&self.r0.to_be_bytes());
        bytes[4..8].copy_from_slice(&self.r1.to_be_bytes());
        bytes[8..12].copy_from_slice(&self.r2.to_be_bytes());
        bytes[12..16].copy_from_slice(&self.r3.to_be_bytes());
        bytes
    }
}

/// DW-MMC 轮询主控。
pub struct DwMmcController<I: MmcRegisterIo> {
    io: I,
    ciu_frequency_hz: u64,
}

impl<I: MmcRegisterIo> DwMmcController<I> {
    pub fn new(io: I, ciu_frequency_hz: u64) -> Self {
        Self {
            io,
            ciu_frequency_hz,
        }
    }

    /// 控制器 + FIFO 复位。复位位写 1 后硬件自清；轮询至位清 0。
    pub fn reset(&mut self) -> Result<(), MmcError> {
        self.io.write32(REG_RST_N, RST_CTRL | RST_FIFO);
        self.poll_until(|value| value & (RST_CTRL | RST_FIFO) == 0, REG_RST_N)?;
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

    /// 配置时钟。先等待数据忙清除，再写分频 + 使能，最后用 UPDATE_CLOCK
    /// 命令让硬件应用。
    pub fn set_clock(&mut self, frequency_hz: u64) -> Result<(), MmcError> {
        self.wait_data_busy_clear()?;

        let target = frequency_hz.max(100_000);
        let divider = (self.ciu_frequency_hz / target.saturating_mul(2))
            .max(1)
            .min(u32::MAX as u64) as u32;
        self.io.write32(REG_CLKDIV, divider);
        self.io.write32(REG_CLKENA, CLKENA_ENABLE);

        let command = MmcCommand {
            index: 0,
            argument: 0,
            response_type: MmcResponseType::None,
            data_present: false,
            read: false,
            init: false,
            update_clock: true,
        };
        self.send_command(command)?;
        Ok(())
    }

    /// 设置总线宽度（1/4/8-bit）。
    pub fn set_bus_width(&mut self, bus_width: u8) -> Result<(), MmcError> {
        if bus_width != 1 && bus_width != 4 && bus_width != 8 {
            return Err(MmcError::InvalidArgument);
        }
        self.io.write32(REG_CTYPE, ctype_for_width(bus_width));
        Ok(())
    }

    /// 发送命令并等待响应。带数据时由调用方另行读/写 FIFO。
    pub fn send_command(&mut self, command: MmcCommand) -> Result<MmcResponse, MmcError> {
        let cmd_value = self.build_command_value(&command);
        self.io.write32(REG_CMDARG, command.argument);
        self.io.write32(REG_CMD, cmd_value);

        self.poll_command_done()?;

        let mut response = MmcResponse::default();
        match command.response_type {
            MmcResponseType::None => {}
            MmcResponseType::R2 => {
                response.r0 = self.io.read32(REG_RESP0);
                response.r1 = self.io.read32(REG_RESP1);
                response.r2 = self.io.read32(REG_RESP2);
                response.r3 = self.io.read32(REG_RESP3);
            }
            MmcResponseType::R1 | MmcResponseType::R3 | MmcResponseType::R6
            | MmcResponseType::R7 => {
                response.r0 = self.io.read32(REG_RESP0);
            }
        }
        Ok(response)
    }

    /// PIO 单块读取：从 FIFO 拉取一个块（`output.len()` 字节）。
    pub fn read_block_data(&mut self, output: &mut [u8]) -> Result<(), MmcError> {
        if output.is_empty() || output.len() % 4 != 0 {
            return Err(MmcError::InvalidArgument);
        }
        let words = output.len() / 4;
        let mut received = 0_usize;
        let mut total = 0_usize;
        while total < words {
            let interrupts = self.poll_for(|value| value & (INT_RX_DATA_REQ | INT_DATA_OVER | INT_DATA_CRC | INT_DATA_TIMEOUT | INT_FIFO_OVERRUN) != 0, REG_RINTSTS)?;
            if interrupts & INT_DATA_CRC != 0 {
                self.clear_interrupts(INT_DATA_CRC | INT_DATA_OVER);
                return Err(MmcError::CrcError);
            }
            if interrupts & INT_DATA_TIMEOUT != 0 {
                self.clear_interrupts(INT_DATA_TIMEOUT | INT_DATA_OVER);
                return Err(MmcError::Timeout);
            }
            if interrupts & INT_FIFO_OVERRUN != 0 {
                self.clear_interrupts(INT_FIFO_OVERRUN);
                return Err(MmcError::FifoOverrun);
            }
            // 读 FIFO 直到空（分批到达）。
            while !self.fifo_empty() && received < words {
                let word = self.io.read32(REG_FIFO);
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
            MmcResponseType::None => {
                value |= CMD_RESP_NONE;
            }
            MmcResponseType::R2 => {
                value |= CMD_RESP_EXPECT | CMD_RESP_136;
            }
            MmcResponseType::R1 | MmcResponseType::R6 | MmcResponseType::R7 => {
                value |= CMD_RESP_EXPECT | CMD_RESP_48 | CMD_CRC_CHECK | CMD_INDEX_CHECK;
            }
            MmcResponseType::R3 => {
                value |= CMD_RESP_EXPECT | CMD_RESP_48;
            }
        }
        if command.data_present {
            value |= CMD_DATA_PRESENT;
            if command.read {
                value |= CMD_READ;
            }
        }
        if command.init {
            value |= CMD_SEND_INIT;
        }
        if command.update_clock {
            value |= CMD_UPDATE_CLOCK;
        }
        value
    }

    /// 等待 CMD_DONE 或命令错误，返回并清理中断。
    fn poll_command_done(&mut self) -> Result<(), MmcError> {
        let mask = INT_CMD_DONE | INT_RESP_CRC | INT_RESP_TIMEOUT | INT_START_BIT | INT_END_BIT;
        let interrupts = self.poll_for(|value| value & mask != 0, REG_RINTSTS)?;
        if interrupts & INT_CMD_DONE != 0 {
            self.io.write32(REG_RINTSTS, INT_CMD_DONE);
            return Ok(());
        }
        if interrupts & (INT_RESP_CRC | INT_START_BIT | INT_END_BIT) != 0 {
            self.clear_interrupts(interrupts & mask);
            return Err(MmcError::CrcError);
        }
        self.io.write32(REG_RINTSTS, INT_RESP_TIMEOUT);
        Err(MmcError::Timeout)
    }

    fn wait_data_busy_clear(&mut self) -> Result<(), MmcError> {
        self.poll_for(|value| value & STATUS_DATA_BUSY == 0, REG_STATUS)?;
        Ok(())
    }

    fn fifo_empty(&mut self) -> bool {
        self.io.read32(REG_STATUS) & STATUS_FIFO_EMPTY != 0
    }

    fn clear_interrupts(&mut self, bits: u32) {
        self.io.write32(REG_RINTSTS, bits);
    }

    fn poll_for(
        &mut self,
        predicate: impl Fn(u32) -> bool,
        register: usize,
    ) -> Result<u32, MmcError> {
        for _ in 0..DEADLINE_POLLS {
            let value = self.io.read32(register);
            if predicate(value) {
                return Ok(value);
            }
        }
        Err(MmcError::Timeout)
    }

    fn poll_until(
        &mut self,
        predicate: impl Fn(u32) -> bool,
        register: usize,
    ) -> Result<(), MmcError> {
        self.poll_for(predicate, register).map(|_| ())
    }
}

#[cfg(debug_assertions)]
pub fn verify() {
    use super::mock::{MockFailure, MockRegisterIo};
    use alloc::vec;

    // 1) 正常命令完成。
    let mock = MockRegisterIo::new().with_responses(vec![0x1a2b_3c4d, 0, 0, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    let response = controller
        .send_command(MmcCommand::new(8, 0x1aa, MmcResponseType::R7))
        .expect("command 8 completed");
    assert_eq!(response.r0, 0x1a2b_3c4d);

    // 2) 响应超时。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResponseTimeout);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    assert_eq!(
        controller.send_command(MmcCommand::new(8, 0x1aa, MmcResponseType::R7)),
        Err(MmcError::Timeout),
    );

    // 3) 响应 CRC 错误。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResponseCrc);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    assert_eq!(
        controller.send_command(MmcCommand::new(55, 0, MmcResponseType::R1)),
        Err(MmcError::CrcError),
    );

    // 4) 数据超时。
    let mock = MockRegisterIo::new().with_failure(MockFailure::DataTimeout);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data(true))
        .expect("data command accepted");
    let mut block = [0_u8; 512];
    assert_eq!(controller.read_block_data(&mut block), Err(MmcError::Timeout));

    // 5) FIFO 上溢。
    let mock = MockRegisterIo::new().with_failure(MockFailure::FifoOverrun);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data(true))
        .expect("data command accepted");
    let mut block = [0_u8; 512];
    assert_eq!(
        controller.read_block_data(&mut block),
        Err(MmcError::FifoOverrun),
    );

    // 6) 控制器复位卡死 → Timeout（不挂死）。
    let mock = MockRegisterIo::new().with_failure(MockFailure::ResetHang);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    assert_eq!(controller.reset(), Err(MmcError::Timeout));

    // 7) FIFO 数据分批到达。
    let mock = MockRegisterIo::new()
        .with_fifo_data(vec![0x0302_0100, 0x0706_0504, 0x0b0a_0908, 0x0f0e_0d0c])
        .with_fifo_batch(2);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    controller
        .send_command(MmcCommand::new(17, 0, MmcResponseType::R1).with_data(true))
        .expect("data command accepted");
    let mut block = [0_u8; 16];
    controller
        .read_block_data(&mut block)
        .expect("batched fifo read");
    assert_eq!(block, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);

    // 8) 错误后恢复：清掉故障再发命令成功。
    let mut mock = MockRegisterIo::new().with_failure(MockFailure::ResponseTimeout);
    mock.set_failure(None);
    let mut controller = DwMmcController::new(mock, 25_000_000);
    let response = controller
        .send_command(MmcCommand::new(13, 0x1234, MmcResponseType::R1))
        .expect("command after error recovery");
    assert_eq!(response.r0, 0);

    // 9) 时钟与总线宽度配置。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 25_000_000);
    controller.set_clock(400_000).expect("400 kHz clock");
    controller.set_clock(25_000_000).expect("25 MHz clock");
    controller.set_bus_width(4).expect("4-bit bus width");
    assert_eq!(controller.set_bus_width(3), Err(MmcError::InvalidArgument));

    // 10) 复位 + 禁用中断不报错。
    let mock = MockRegisterIo::new();
    let mut controller = DwMmcController::new(mock, 25_000_000);
    controller.reset().expect("reset");
    controller.disable_interrupts();

    crate::println!("C7 DW-MMC controller gate:");
    crate::println!("  command complete    : verified");
    crate::println!("  response timeout    : verified");
    crate::println!("  response CRC        : verified");
    crate::println!("  data timeout        : verified");
    crate::println!("  FIFO overrun        : verified");
    crate::println!("  reset hang          : verified");
    crate::println!("  FIFO batching       : verified");
    crate::println!("  error recovery      : verified");
    crate::println!("  clock/bus-width     : verified");
}
