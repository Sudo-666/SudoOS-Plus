//! SD 卡只读块设备（注册为 `/dev/mmcblk1`，CodePlan C8）。
//!
//! 控制器（含 mock 可测）包在自旋锁里，`read_block` 通过 CMD17 + PIO FIFO
//! 单块读取。SDSC 用字节地址、SDHC/SDXC 用块号；写入一律拒绝。

use alloc::sync::Arc;

use crate::{
    block::{self, BlockDevice, BlockError},
    irq_lock::IrqSpinLock,
    lockdep::{LockClass, LockRank},
};

use super::dw_mmc::{DwMmcController, MmcCommand, MmcError, MmcRegisterIo, MmcResponseType};
use super::sd::{SD_CMD17, SdCardInfo};

const SD_BLOCK_LOCK: LockClass = LockClass::new("mmc.block", LockRank::Vfs, 21);
const SD_BLOCK_SIZE: usize = 512;

/// 只读 SD 卡块设备。
pub struct SdBlockDevice<I: MmcRegisterIo> {
    controller: IrqSpinLock<DwMmcController<I>>,
    info: SdCardInfo,
}

impl<I: MmcRegisterIo> SdBlockDevice<I> {
    pub fn new(controller: DwMmcController<I>, info: SdCardInfo) -> Result<Self, BlockError> {
        if info.block_count == 0 {
            return Err(BlockError::InvalidArgument);
        }
        Ok(Self {
            controller: IrqSpinLock::new_with_class(controller, SD_BLOCK_LOCK),
            info,
        })
    }

    fn read_single_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        if block >= self.info.block_count {
            return Err(BlockError::OutOfRange);
        }
        if output.len() != SD_BLOCK_SIZE {
            return Err(BlockError::BufferTooSmall);
        }
        let address = if self.info.is_sdhc {
            block
        } else {
            block
                .checked_mul(SD_BLOCK_SIZE as u64)
                .ok_or(BlockError::AddressOverflow)?
        };
        let argument = u32::try_from(address).map_err(|_| BlockError::AddressOverflow)?;

        let mut controller = self.controller.lock();
        controller
            .send_command(MmcCommand::new(SD_CMD17, argument, MmcResponseType::R1).with_data(true))
            .map_err(mmc_to_block)?;
        controller.read_block_data(output).map_err(mmc_to_block)
    }
}

impl<I: MmcRegisterIo> BlockDevice for SdBlockDevice<I> {
    fn block_size(&self) -> usize {
        SD_BLOCK_SIZE
    }

    fn block_count(&self) -> u64 {
        self.info.block_count
    }

    fn read_block(&self, block: u64, output: &mut [u8]) -> Result<(), BlockError> {
        self.read_single_block(block, output)
    }

    fn write_block(&self, _block: u64, _input: &[u8]) -> Result<(), BlockError> {
        Err(BlockError::DeviceReadOnly)
    }

    fn is_read_only(&self) -> bool {
        true
    }
}

/// 把初始化好的 SD 卡注册为 `/dev/mmcblk1`。
pub fn register_mmcblk1<I: MmcRegisterIo>(
    controller: DwMmcController<I>,
    info: SdCardInfo,
) -> Result<(), BlockError> {
    let device = Arc::new(SdBlockDevice::new(controller, info)?);
    block::register_device("mmcblk1", device as Arc<dyn BlockDevice>)
}

fn mmc_to_block(error: MmcError) -> BlockError {
    match error {
        MmcError::Timeout | MmcError::CrcError => BlockError::InvalidArgument,
        MmcError::FifoUnderrun | MmcError::FifoOverrun => BlockError::InvalidArgument,
        MmcError::ResetFailed | MmcError::NotReady | MmcError::Io => BlockError::InvalidArgument,
        MmcError::InvalidArgument => BlockError::InvalidArgument,
    }
}
