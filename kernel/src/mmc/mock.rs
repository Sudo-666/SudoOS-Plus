//! DW-MMC 控制器 mock（单元测试用，`#[cfg(debug_assertions)]`）。
//!
//! K3.3：与修正后的寄存器语义一致——`CMD_START` 必须置位，带数据命令先写
//! `BLKSIZ/BYTCNT`，复位走 `CTRL` 位，FIFO 偏移按 `VERID` 判定
//! （`0x100` / `0x200`）。写入 `CMD` 时按配置模拟完成/错误；为数据读命令
//! 提供 FIFO 数据，支持分批到达（`fifo_batch`）。寄存器数组按 4 字节偏移
//! 索引，仅覆盖常规寄存器区（0x000..0x0fc）；FIFO 读走独立路径。

use alloc::vec::Vec;

use super::dw_mmc::MmcRegisterIo;
use super::registers::*;

/// mock 故障注入。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MockFailure {
    /// 命令响应超时。
    ResponseTimeout,
    /// 响应 CRC 错误。
    ResponseCrc,
    /// 数据超时。
    DataTimeout,
    /// FIFO 上溢。
    FifoOverrun,
    /// 控制器复位卡死（CTRL 复位位永不自清）。
    ResetHang,
    /// 命令永不完成（CMD_START 不自清，无完成信号）。
    CommandHang,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MockCommandTrace {
    pub index: u8,
    pub argument: u32,
    pub data_present: bool,
    pub read: bool,
    pub update_clock: bool,
    pub cmd_start: bool,
    /// 带数据命令的字节数（来自 BLKSIZ/BYTCNT 编程）。
    pub data_length: Option<usize>,
    /// 带数据命令的 FIFOTH 值（水印须低于 FIFO 深度，K3.4）。
    pub fifo_threshold: Option<u32>,
}

/// 模拟寄存器。寄存器数组按 `offset / 4` 索引。
pub struct MockRegisterIo {
    regs: [u32; 64],
    /// 每条命令的 RESP0..3（命令 N 用 `responses[4N..4N+4]`）。
    responses: Vec<u32>,
    response_index: usize,
    failure: Option<MockFailure>,
    /// 仅作用于指定命令索引的故障（用后即清）。
    one_shot_failure: Option<(MockFailure, u8)>,
    fifo_words: Vec<u32>,
    fifo_index: usize,
    /// 当前 RXDR 窗口已读字数（驱动分批模拟）。
    batch_served: usize,
    /// 每次 RXDR 窗口可见的字数；0 = 一次性全部可见。
    fifo_batch: usize,
    data_active: bool,
    pub commands: Vec<MockCommandTrace>,
}

impl MockRegisterIo {
    pub fn new() -> Self {
        Self {
            regs: [0; 64],
            responses: Vec::new(),
            response_index: 0,
            failure: None,
            one_shot_failure: None,
            fifo_words: Vec::new(),
            fifo_index: 0,
            batch_served: 0,
            fifo_batch: 0,
            data_active: false,
            commands: Vec::new(),
        }
    }

    /// 设置命令响应序列（每条 4 个 u32，对应 RESP0..3）。
    pub fn with_responses(mut self, responses: Vec<u32>) -> Self {
        self.responses = responses;
        self
    }

    /// 设置故障注入。
    pub fn with_failure(mut self, failure: MockFailure) -> Self {
        self.failure = Some(failure);
        self
    }

    /// 设置只对指定命令索引生效一次故障注入（用后即清）。
    pub fn with_failure_once(mut self, failure: MockFailure, command_index: u8) -> Self {
        self.one_shot_failure = Some((failure, command_index));
        self
    }

    /// 设置数据读命令要提供的 FIFO 数据。
    pub fn with_fifo_data(mut self, words: Vec<u32>) -> Self {
        self.fifo_words = words;
        self
    }

    /// 设置每次 RXDR 窗口可见字数（分批到达模拟）。
    pub fn with_fifo_batch(mut self, batch: usize) -> Self {
        self.fifo_batch = batch;
        self
    }

    /// 设置 VERID（决定 mock 的 FIFO 偏移与控制器一致）。
    pub fn with_verid(mut self, verid: u32) -> Self {
        self.regs[Self::index(REG_VERID)] = verid;
        self
    }

    /// 改变故障注入（错误恢复后再发命令的测试）。
    pub fn set_failure(&mut self, failure: Option<MockFailure>) {
        self.failure = failure;
    }

    fn index(offset: usize) -> usize {
        offset / 4
    }

    /// FIFO 偏移：与控制器同规则（VERID ≥ 2.40a → 0x200）。
    fn fifo_offset(&self) -> usize {
        let verid = self.regs[Self::index(REG_VERID)] & 0xffff;
        if verid >= DW_MMC_240A {
            DATA_240A_OFFSET
        } else {
            DATA_OFFSET
        }
    }

    /// STATUS 的 FIFO 空位：数据耗尽，或分批模式下当前批次已读完
    /// （驱动读空一次 RXDR 窗口后，清除 RXDR 会让 mock 重新载入下一批）。
    fn fifo_empty_now(&self) -> bool {
        if self.fifo_index >= self.fifo_words.len() {
            return true;
        }
        if self.fifo_batch > 0 {
            return self.batch_served > 0 && self.batch_served % self.fifo_batch == 0;
        }
        false
    }
}

impl MmcRegisterIo for MockRegisterIo {
    fn read32(&mut self, offset: usize) -> u32 {
        let index = Self::index(offset);
        match offset {
            REG_RESP0 | REG_RESP1 | REG_RESP2 | REG_RESP3 => {
                let resp_index = (offset - REG_RESP0) / 4;
                self.responses
                    .get(self.response_index.saturating_sub(1) * 4 + resp_index)
                    .copied()
                    .unwrap_or(0)
            }
            // FIFO 偏移按 VERID 判定（0x100 / 0x200），独立于 regs 数组。
            _ if offset == self.fifo_offset() => {
                if self.fifo_index < self.fifo_words.len() {
                    let word = self.fifo_words[self.fifo_index];
                    self.fifo_index += 1;
                    self.batch_served += 1;
                    word
                } else {
                    0
                }
            }
            REG_STATUS => {
                // FCNT 域：空 → 0（fifo_empty），非空 → 1。
                if self.fifo_empty_now() {
                    0
                } else {
                    1 << STATUS_FIFO_COUNT_SHIFT
                }
            }
            REG_CTRL => {
                if self.failure == Some(MockFailure::ResetHang) {
                    CTRL_RESET | CTRL_FIFO_RESET
                } else {
                    self.regs[index]
                }
            }
            _ => self.regs[index],
        }
    }

    fn write32(&mut self, offset: usize, value: u32) {
        let index = Self::index(offset);
        match offset {
            REG_CTRL => {
                // 复位位写 1 后硬件自清；卡死故障下保持置位。
                if self.failure == Some(MockFailure::ResetHang) {
                    self.regs[index] = value & (CTRL_RESET | CTRL_FIFO_RESET);
                } else {
                    self.regs[index] = value & !(CTRL_RESET | CTRL_FIFO_RESET);
                }
            }
            REG_CMDARG => {
                self.regs[index] = value;
            }
            REG_CMD => {
                let data_present = value & CMD_DATA_EXPECTED != 0;
                let command = MockCommandTrace {
                    index: (value & CMD_INDEX_MASK) as u8,
                    argument: self.regs[Self::index(REG_CMDARG)],
                    data_present,
                    // DAT_WR 未置位 = 主机读（卡 → 主机）。
                    read: data_present && value & CMD_DATA_WRITE == 0,
                    update_clock: value & CMD_UPDATE_CLOCK != 0,
                    cmd_start: value & CMD_START != 0,
                    data_length: if data_present {
                        Some(self.regs[Self::index(REG_BYTCNT)] as usize)
                    } else {
                        None
                    },
                    fifo_threshold: if data_present {
                        Some(self.regs[Self::index(REG_FIFOTH)])
                    } else {
                        None
                    },
                };
                self.commands.push(command);
                self.response_index += 1;

                let command_index = (value & CMD_INDEX_MASK) as u8;
                let active_failure = match self.one_shot_failure {
                    Some((failure, index)) if index == command_index => {
                        self.one_shot_failure = None;
                        Some(failure)
                    }
                    _ => self.failure,
                };
                // 命令执行完成 → 硬件自清 CMD_START（K3.4）。CommandHang
                // 故障下保持置位，模拟命令永不完成。
                self.regs[index] = match active_failure {
                    Some(MockFailure::CommandHang) => value,
                    _ => value & !CMD_START,
                };

                let rintsts = &mut self.regs[Self::index(REG_RINTSTS)];
                match active_failure {
                    Some(MockFailure::ResponseTimeout) => {
                        *rintsts |= INT_RESP_TIMEOUT;
                    }
                    Some(MockFailure::ResponseCrc) => {
                        *rintsts |= INT_RESP_CRC;
                    }
                    Some(MockFailure::DataTimeout) if data_present => {
                        // 命令本身完成，仅数据阶段超时。
                        *rintsts |= INT_CMD_DONE | INT_DATA_TIMEOUT;
                    }
                    Some(MockFailure::FifoOverrun) if data_present => {
                        *rintsts |= INT_CMD_DONE | INT_FIFO_OVERRUN;
                    }
                    Some(MockFailure::CommandHang) => {
                        // 无完成信号。
                    }
                    _ => {
                        // 常规命令置 CMD_DONE；UPDATE_CLOCK 不置——真实硬件
                        // 仅靠 CMD_START 自清表示完成（K3.4），驱动必须能
                        // 在这种信号下返回。
                        if value & CMD_UPDATE_CLOCK == 0 {
                            *rintsts |= INT_CMD_DONE;
                        }
                        if data_present && command.read {
                            self.data_active = true;
                            self.fifo_index = 0;
                            self.batch_served = 0;
                            if self.fifo_words.is_empty() {
                                *rintsts |= INT_DATA_OVER;
                                self.data_active = false;
                            } else {
                                *rintsts |= INT_RX_DATA_REQ;
                            }
                        }
                    }
                }
            }
            REG_RINTSTS => {
                self.regs[index] &= !value;
                // 控制器清掉 RXDR 后：若仍有数据则重新置 RXDR（分批）；
                // 若数据耗尽则置 DATA_OVER 并结束数据阶段。
                if self.data_active {
                    if self.fifo_index >= self.fifo_words.len() {
                        self.regs[index] |= INT_DATA_OVER;
                        self.regs[index] &= !INT_RX_DATA_REQ;
                        self.data_active = false;
                    } else if self.regs[index] & INT_RX_DATA_REQ == 0 {
                        self.regs[index] |= INT_RX_DATA_REQ;
                        self.batch_served = 0;
                    }
                }
            }
            _ => {
                self.regs[index] = value;
            }
        }
    }
}
