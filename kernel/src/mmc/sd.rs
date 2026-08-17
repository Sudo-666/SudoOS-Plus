//! SD 卡初始化与只读块（CodePlan C8）。
//!
//! 在 [`MmcHost`] 之上实现 SD 卡握手（CMD0/CMD8/ACMD41/CMD2/CMD3/CMD9/
//! CMD7/CMD16/ACMD51/ACMD6/CMD13），解析 CSD v1/v2 容量与 SCR 总线宽度，
//! 支持 SDSC 字节寻址与 SDHC/SDXC 块寻址。无卡或初始化失败返回错误，
//! 不 panic。

use super::dw_mmc::{MmcCommand, MmcError, MmcHost, MmcResponse, MmcResponseType};

#[cfg(debug_assertions)]
use alloc::{vec, vec::Vec};

const SD_CMD0: u8 = 0;
const SD_CMD8: u8 = 8;
const SD_CMD55: u8 = 55;
const SD_ACMD41: u8 = 41;
const SD_CMD2: u8 = 2;
const SD_CMD3: u8 = 3;
const SD_CMD9: u8 = 9;
const SD_CMD7: u8 = 7;
const SD_CMD16: u8 = 16;
const SD_ACMD51: u8 = 51;
const SD_ACMD6: u8 = 6;
const SD_CMD13: u8 = 13;
pub const SD_CMD17: u8 = 17;

/// CMD8 检查模式（电压 2.7-3.6V + 检查字节 0xAA）。
const SD_IF_COND_PATTERN: u32 = 0x1aa;
const SD_ACMD41_ARG: u32 = 0x40ff_8000; // HCS + 全电压窗口
const SD_OCR_CCS: u32 = 1 << 30;
const SD_OCR_POWER_UP: u32 = 1 << 31;
const SD_ACMD6_ARG_4BIT: u32 = 2;
const SD_BLOCK_LEN: u32 = 512;
/// ACMD41 OCR 轮询：总 deadline ~1s、每轮间隔 5ms（卡上电需数百 ms），
/// 叠加次数上限兜底计时器异常（避免死循环）。
const ACMD41_POLL_DEADLINE_MS: u64 = 1_000;
const ACMD41_POLL_INTERVAL_MS: u64 = 5;
const MAX_ACMD41_RETRIES: usize = 512;

/// 初始化后的 SD 卡信息。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdCardInfo {
    /// 相对卡地址（RCA）。
    pub rca: u32,
    /// SDHC/SDXC 块寻址（否则 SDSC 字节寻址）。
    pub is_sdhc: bool,
    /// 512 字节块数。
    pub block_count: u64,
    /// 最终总线宽度（1 或 4）。
    pub bus_width: u8,
    /// CID（16 字节）。
    pub cid: [u8; 16],
}

/// 初始化 SD 卡。`host` 必须先复位并配置好 400 kHz 初始化时钟。
pub fn initialize_card<H: MmcHost>(host: &mut H) -> Result<SdCardInfo, MmcError> {
    host.set_bus_width(1)?;

    // CMD0：进入空闲态（初始化命令）。
    host.send_command(MmcCommand {
        index: SD_CMD0,
        argument: 0,
        response_type: MmcResponseType::None,
        data_present: false,
        read: false,
        init: true,
        update_clock: false,
        data_length: None,
    })?;

    // CMD8：SD v2 接口条件。旧卡（v1）不响应 → Timeout → 视为 SDSC。
    let interface_condition = match host.send_command(MmcCommand::new(
        SD_CMD8,
        SD_IF_COND_PATTERN,
        MmcResponseType::R7,
    )) {
        Ok(response) => response,
        Err(MmcError::Timeout) => {
            crate::println!("mmc-sd: CMD8 no response — SD v1 (SDSC) card");
            MmcResponse::default()
        }
        Err(error) => return Err(error),
    };
    let is_v2 = interface_condition.r0 & 0x0fff == SD_IF_COND_PATTERN;

    // CMD55 + ACMD41：OCR 轮询，power-up busy 位（bit31）就绪后读取 CCS。
    // 总 deadline ~1s，每轮间隔 5ms（上电需数百 ms），叠加次数上限兜底。
    let mut is_sdhc = false;
    let mut powered_up = false;
    let freq_hz = crate::time::clock_frequency_hz();
    let deadline = crate::time::now()
        .cycles()
        .saturating_add(freq_hz / 1000 * ACMD41_POLL_DEADLINE_MS);
    let mut retries = 0;
    while !powered_up {
        host.send_command(MmcCommand::new(SD_CMD55, 0, MmcResponseType::R1))?;
        let ocr = host.send_command(MmcCommand::new(
            SD_ACMD41,
            SD_ACMD41_ARG,
            MmcResponseType::R3,
        ))?;
        if ocr.r0 & SD_OCR_POWER_UP != 0 {
            is_sdhc = is_v2 && ocr.r0 & SD_OCR_CCS != 0;
            powered_up = true;
            break;
        }
        retries += 1;
        if retries >= MAX_ACMD41_RETRIES || crate::time::now().cycles() >= deadline {
            break;
        }
        // 1-10ms 间隔忙等，避免急转供电/总线。
        let interval = crate::time::now()
            .cycles()
            .saturating_add(freq_hz / 1000 * ACMD41_POLL_INTERVAL_MS);
        while crate::time::now().cycles() < interval {
            core::hint::spin_loop();
        }
    }
    if !powered_up {
        crate::println!("mmc-sd: ACMD41 busy timeout");
        return Err(MmcError::Timeout);
    }

    // CMD2：CID。
    let cid_response = host.send_command(MmcCommand::new(SD_CMD2, 0, MmcResponseType::R2))?;
    let cid = cid_response.card_data();

    // CMD3：RCA。
    let rca_response = host.send_command(MmcCommand::new(SD_CMD3, 0, MmcResponseType::R6))?;
    let rca = rca_response.r0 >> 16;
    if rca == 0 {
        return Err(MmcError::InvalidArgument);
    }

    // CMD9：CSD → 容量。
    let csd_response = host.send_command(MmcCommand::new(
        SD_CMD9,
        rca << 16,
        MmcResponseType::R2,
    ))?;
    let csd = csd_response.card_data();
    let (block_count, block_size) = parse_csd(&csd, is_sdhc)?;
    if block_count == 0 {
        return Err(MmcError::InvalidArgument);
    }

    // CMD7：选中该卡（R1b，响应后等 busy 清才能发 CMD55/ACMD51）。
    host.send_command(MmcCommand::new(
        SD_CMD7,
        rca << 16,
        MmcResponseType::R1b,
    ))?;

    // CMD16：仅 SDSC 需要设置块长。
    if !is_sdhc {
        host.send_command(MmcCommand::new(
            SD_CMD16,
            SD_BLOCK_LEN,
            MmcResponseType::R1,
        ))?;
    }

    // ACMD51：SCR（8 字节）→ 总线宽度支持。
    host.send_command(MmcCommand::new(SD_CMD55, rca << 16, MmcResponseType::R1))?;
    host.send_command(MmcCommand::new(
        SD_ACMD51,
        0,
        MmcResponseType::R1,
    )
    .with_data_length(true, 8))?;
    let mut scr = [0_u8; 8];
    host.read_block_data(&mut scr)?;

    let mut bus_width = 1;
    if scr_supports_4bit(&scr) {
        bus_width = 4;
        // ACMD6：切 4-bit。
        host.send_command(MmcCommand::new(SD_CMD55, rca << 16, MmcResponseType::R1))?;
        host.send_command(MmcCommand::new(
            SD_ACMD6,
            SD_ACMD6_ARG_4BIT,
            MmcResponseType::R1,
        ))?;
        host.set_bus_width(4)?;
    } else {
        crate::println!("mmc-sd: SCR reports no 4-bit support — staying 1-bit");
    }

    // CMD13：状态确认。
    host.send_command(MmcCommand::new(
        SD_CMD13,
        rca << 16,
        MmcResponseType::R1,
    ))?;

    // 切工作时钟。
    host.set_clock(25_000_000)?;

    crate::println!(
        "mmc-sd: card ready rca={:#x} sdhc={} blocks={} ({:.1} MiB) bus-width={}",
        rca,
        is_sdhc,
        block_count,
        block_count as f64 * block_size as f64 / (1024.0 * 1024.0),
        bus_width,
    );

    Ok(SdCardInfo {
        rca,
        is_sdhc,
        block_count,
        bus_width,
        cid,
    })
}

/// 解析 CSD 容量。返回 `(512 字节块数, 块大小)`。
/// CSD v2（SDHC/SDXC）：`blocks = (C_SIZE + 1) * 1024`，块 512。
/// CSD v1（SDSC）：`blocks = (C_SIZE + 1) << (C_SIZE_MULT + 2)`，块 `1 << READ_BL_LEN`。
pub fn parse_csd(csd: &[u8; 16], is_sdhc: bool) -> Result<(u64, u64), MmcError> {
    if is_sdhc {
        let c_size = csd_field(csd, 69, 48);
        let blocks = c_size
            .checked_add(1)
            .and_then(|value| value.checked_mul(1024))
            .ok_or(MmcError::InvalidArgument)?;
        Ok((blocks, SD_BLOCK_LEN as u64))
    } else {
        let read_bl_len = csd_field(csd, 83, 80);
        let c_size = csd_field(csd, 73, 62);
        let c_size_mult = csd_field(csd, 49, 47);
        let block_size = 1_u64
            .checked_shl(read_bl_len as u32)
            .ok_or(MmcError::InvalidArgument)?;
        let multiplier = 1_u64
            .checked_shl(c_size_mult.checked_add(2).ok_or(MmcError::InvalidArgument)? as u32)
            .ok_or(MmcError::InvalidArgument)?;
        let byte_capacity = c_size
            .checked_add(1)
            .and_then(|value| value.checked_mul(multiplier))
            .and_then(|value| value.checked_mul(block_size))
            .ok_or(MmcError::InvalidArgument)?;
        let blocks = byte_capacity / SD_BLOCK_LEN as u64;
        if blocks == 0 {
            return Err(MmcError::InvalidArgument);
        }
        Ok((blocks, block_size))
    }
}

/// SCR 是否支持 4-bit：SCR bits 48-55 = SD_BUS_WIDTHS，bit 50 为 4-bit。
fn scr_supports_4bit(scr: &[u8; 8]) -> bool {
    scr[1] & 0x04 != 0
}

/// 从 16 字节大端 CSD 中取 `[hi, lo]` 位区间（bit 127 = CSD 首字节 MSB）。
fn csd_field(csd: &[u8; 16], hi: usize, lo: usize) -> u64 {
    let mut value = 0_u64;
    for bit in (lo..=hi).rev() {
        let byte = 15 - bit / 8;
        let bit_in_byte = 7 - (bit % 8);
        value = (value << 1) | u64::from((csd[byte] >> bit_in_byte) & 1);
    }
    value
}

#[cfg(debug_assertions)]
pub fn verify() {
    use super::dw_mmc::{DwMmcController, MmcCommand, MmcResponseType};
    use super::mock::{MockFailure, MockRegisterIo};
    use alloc::{vec, vec::Vec};

    // SDHC v2 初始化响应序列（每条命令 4 个 RESP 字）。
    // CMD0(无), CMD8(R7=0x1aa), CMD55(R1), ACMD41(R3 busy),
    // CMD55, ACMD41(R3 ready+CCS), CMD2(R2 CID), CMD3(R6 RCA),
    // CMD9(R2 CSD v2), CMD7(R1), CMD55, ACMD51(R1), CMD55, ACMD6(R1), CMD13(R1)
    let mut responses: Vec<u32> = Vec::new();
    responses.extend([0, 0, 0, 0]); // CMD0
    responses.extend([0x1aa, 0, 0, 0]); // CMD8
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0x00ff_8000, 0, 0, 0]); // ACMD41 busy
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0xc0ff_8000, 0, 0, 0]); // ACMD41 ready + CCS
    // CMD2 CID：非回文字节序列（区分 BE/LE 字节序）。DW-MMC 逆序存储——
    // RESP3 = 协议最高 word（payload[0..4]）、RESP0 = 最低 word
    // （payload[12..16]），字内大端。mock 按 [RESP0, RESP1, RESP2, RESP3]
    // 编码，card_data() 逆序读 + to_be_bytes() 还原，往返应得到原始 16 字节。
    let cid_payload = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];
    responses.extend([
        u32::from_be_bytes([cid_payload[12], cid_payload[13], cid_payload[14], cid_payload[15]]),
        u32::from_be_bytes([cid_payload[8], cid_payload[9], cid_payload[10], cid_payload[11]]),
        u32::from_be_bytes([cid_payload[4], cid_payload[5], cid_payload[6], cid_payload[7]]),
        u32::from_be_bytes([cid_payload[0], cid_payload[1], cid_payload[2], cid_payload[3]]),
    ]); // CMD2 CID
    responses.extend([0x1234_0000, 0, 0, 0]); // CMD3 RCA 0x1234
    responses.extend(csd_v2_words(0x12345)); // CMD9 CSD v2, C_SIZE=0x12345
    responses.extend([0, 0, 0, 0]); // CMD7
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD51
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD6
    responses.extend([0, 0, 0, 0]); // CMD13

    // SCR：byte0 = 结构版本 0x00, byte1 = SD_BUS_WIDTHS 0x04（支持 4-bit）。
    // mock 以小端把字放进输出字节数组，故 word0 的 bits 8-15 = 0x04。
    let scr_words = vec![0x0000_0400u32, 0];
    let mock = MockRegisterIo::new()
        .with_responses(responses)
        .with_fifo_data(scr_words);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);

    let info = initialize_card(&mut controller).expect("SDHC init");
    assert!(info.is_sdhc);
    assert_eq!(info.bus_width, 4);
    assert_eq!(info.rca, 0x1234);
    // CSD v2: blocks = (0x12345 + 1) * 1024
    assert_eq!(info.block_count, (0x12345 + 1) * 1024);
    // CID 字节序：RESP3..RESP0 大端编码必须被 card_data() 正确还原。
    assert_eq!(
        info.cid, cid_payload,
        "R2 must decode RESP3..RESP0 as big-endian protocol payload"
    );

    // 2) SDSC v1 初始化（CMD8 超时，无 4-bit）。
    let mut responses: Vec<u32> = Vec::new();
    responses.extend([0, 0, 0, 0]); // CMD0
    responses.extend([0, 0, 0, 0]); // CMD8(超时后 mock 也计入一条响应)
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0x80ff_8000, 0, 0, 0]); // ACMD41 ready（无 CCS）
    responses.extend([0, 0, 0, 0]); // CMD2
    responses.extend([0x4321_0000, 0, 0, 0]); // CMD3 RCA
    responses.extend(csd_v1_words()); // CMD9 CSD v1
    responses.extend([0, 0, 0, 0]); // CMD7
    responses.extend([0, 0, 0, 0]); // CMD16
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD51
    responses.extend([0, 0, 0, 0]); // CMD13

    let scr_words = vec![0x0000_0000u32, 0]; // 无 4-bit
    let mock = MockRegisterIo::new()
        .with_failure_once(MockFailure::ResponseTimeout, 8) // 仅 CMD8 超时
        .with_responses(responses)
        .with_fifo_data(scr_words);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let info = initialize_card(&mut controller).expect("SDSC init");
    assert!(!info.is_sdhc);
    assert_eq!(info.bus_width, 1);

    // 3) 非法 RCA（CMD3 返回 0）→ InvalidArgument。
    let mut responses: Vec<u32> = Vec::new();
    responses.extend([0, 0, 0, 0]); // CMD0
    responses.extend([0x1aa, 0, 0, 0]); // CMD8
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0xc0ff_8000, 0, 0, 0]); // ACMD41 ready
    responses.extend([0, 0, 0, 0]); // CMD2
    responses.extend([0, 0, 0, 0]); // CMD3 RCA=0 → invalid
    let mock = MockRegisterIo::new().with_responses(responses);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    assert_eq!(initialize_card(&mut controller), Err(MmcError::InvalidArgument));

    // 4) CSD 容量：全零（0 块）被拒；最大合法值不溢出且正确。
    assert_eq!(
        parse_csd(&[0; 16], false),
        Err(MmcError::InvalidArgument),
        "zero-capacity CSD must be rejected",
    );
    assert_eq!(
        parse_csd(&[0xff; 16], true),
        Ok((0x1_0000_0000, 512)),
        "max CSD v2 C_SIZE must not overflow",
    );

    // 5) CMD17 地址计算：SDHC 用块号，SDSC 用字节地址。
    // SDHC：initialize 后向 mock 检查 CMD17 参数。
    let mut responses: Vec<u32> = Vec::new();
    responses.extend([0, 0, 0, 0]); // CMD0
    responses.extend([0x1aa, 0, 0, 0]); // CMD8
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0xc0ff_8000, 0, 0, 0]); // ACMD41 ready+CCS
    responses.extend([0x1111_1111, 0, 0, 0]); // CMD2
    responses.extend([0x1234_0000, 0, 0, 0]); // CMD3
    responses.extend(csd_v2_words(0x1000)); // CMD9
    responses.extend([0, 0, 0, 0]); // CMD7
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD51
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD6
    responses.extend([0, 0, 0, 0]); // CMD13
    let mock = MockRegisterIo::new()
        .with_responses(responses)
        .with_fifo_data(vec![0x0000_0400, 0]);
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let info = initialize_card(&mut controller).expect("SDHC init for block read");

    // 读块 5：SDHC 用块号 5。
    controller
        .send_command(MmcCommand::new(SD_CMD17, 5, MmcResponseType::R1).with_data_length(true, 512))
        .expect("CMD17");
    let cmd17 = controller.io_ref().commands.iter().find(|c| c.index == SD_CMD17);
    assert_eq!(cmd17.expect("CMD17 sent").argument, 5, "SDHC CMD17 must use the block number");
    assert_eq!(cmd17.expect("CMD17 trace").data_length, Some(512));
    assert!(cmd17.expect("CMD17 trace").cmd_start);
    assert!(info.is_sdhc);

    // 6) SDSC CMD17 地址计算：SDSC 用字节地址（块号 × 512）。
    let mut responses: Vec<u32> = Vec::new();
    responses.extend([0, 0, 0, 0]); // CMD0
    responses.extend([0, 0, 0, 0]); // CMD8（超时 → SDSC）
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0x80ff_8000, 0, 0, 0]); // ACMD41 ready（无 CCS）
    responses.extend([0, 0, 0, 0]); // CMD2
    responses.extend([0x4321_0000, 0, 0, 0]); // CMD3 RCA
    responses.extend(csd_v1_words()); // CMD9 CSD v1
    responses.extend([0, 0, 0, 0]); // CMD7
    responses.extend([0, 0, 0, 0]); // CMD16
    responses.extend([0, 0, 0, 0]); // CMD55
    responses.extend([0, 0, 0, 0]); // ACMD51
    responses.extend([0, 0, 0, 0]); // CMD13
    let mock = MockRegisterIo::new()
        .with_failure_once(MockFailure::ResponseTimeout, 8)
        .with_responses(responses)
        .with_fifo_data(vec![0, 0]); // SCR：无 4-bit
    let mut controller = DwMmcController::new(mock, 25_000_000, 32);
    let info = initialize_card(&mut controller).expect("SDSC init for block read");
    assert!(!info.is_sdhc);
    // 读块 7：SDSC 用字节地址 = 7 × 512（换算由 block.rs 的
    // sd_block_address 负责，见 block.rs verify）。
    let byte_address = 7 * 512;
    controller
        .send_command(MmcCommand::new(SD_CMD17, byte_address, MmcResponseType::R1).with_data_length(true, 512))
        .expect("CMD17");
    let cmd17 = controller
        .io_ref()
        .commands
        .iter()
        .find(|c| c.index == SD_CMD17);
    assert_eq!(
        cmd17.expect("CMD17 sent").argument,
        byte_address,
        "SDSC CMD17 argument must carry the byte address"
    );
    assert!(cmd17.expect("CMD17 trace").cmd_start);

    crate::println!("C8 SD card gate:");
    crate::println!("  SDHC init           : verified");
    crate::println!("  SDSC init (no CMD8) : verified");
    crate::println!("  invalid RCA         : verified");
    crate::println!("  CSD overflow        : verified");
    crate::println!("  CMD17 addressing    : verified");
}

/// CSD v2 响应字：构造 C_SIZE 的 16 字节 CSD。
///
/// DW-MMC 逆序存储：RESP3 = 最高 word（payload[0..4]）、RESP0 = 最低
/// word（payload[12..16]），字内大端。mock 按 [RESP0..RESP3] 编码，与
/// `MmcResponse::card_data()` 的逆序读 + `to_be_bytes()` 互为逆运算。
#[cfg(debug_assertions)]
fn csd_v2_words(c_size: u64) -> Vec<u32> {
    // CSD v2：bits 48-69 = C_SIZE。构造 16 字节大端。
    let mut csd = [0_u8; 16];
    for bit in 48..=69u8 {
        let mask = 1_u64 << (bit - 48);
        if c_size & mask != 0 {
            let byte = 15 - bit as usize / 8;
            let bit_in_byte = 7 - (bit as usize % 8);
            csd[byte] |= 1 << bit_in_byte;
        }
    }
    vec![
        u32::from_be_bytes([csd[12], csd[13], csd[14], csd[15]]),
        u32::from_be_bytes([csd[8], csd[9], csd[10], csd[11]]),
        u32::from_be_bytes([csd[4], csd[5], csd[6], csd[7]]),
        u32::from_be_bytes([csd[0], csd[1], csd[2], csd[3]]),
    ]
}

/// CSD v1 响应字：READ_BL_LEN=9 (512), C_SIZE=0x1ff, C_SIZE_MULT=7。
#[cfg(debug_assertions)]
fn csd_v1_words() -> Vec<u32> {
    let mut csd = [0_u8; 16];
    // READ_BL_LEN bits 80-83 = 9
    for (bit, value) in [(80u8, 9u8), (81, 0), (82, 0), (83, 0)] {
        if value & 1 != 0 {
            let byte = 15 - bit as usize / 8;
            let bit_in_byte = 7 - (bit as usize % 8);
            csd[byte] |= 1 << bit_in_byte;
        }
    }
    // C_SIZE bits 62-73 = 0x1ff
    for bit in 62..=73u8 {
        let shifted = bit - 62;
        if 0x1ffu16 & (1 << shifted) != 0 {
            let byte = 15 - bit as usize / 8;
            let bit_in_byte = 7 - (bit as usize % 8);
            csd[byte] |= 1 << bit_in_byte;
        }
    }
    // C_SIZE_MULT bits 47-49 = 7
    for bit in 47..=49u8 {
        let shifted = bit - 47;
        if 7u8 & (1 << shifted) != 0 {
            let byte = 15 - bit as usize / 8;
            let bit_in_byte = 7 - (bit as usize % 8);
            csd[byte] |= 1 << bit_in_byte;
        }
    }
    vec![
        u32::from_be_bytes([csd[12], csd[13], csd[14], csd[15]]),
        u32::from_be_bytes([csd[8], csd[9], csd[10], csd[11]]),
        u32::from_be_bytes([csd[4], csd[5], csd[6], csd[7]]),
        u32::from_be_bytes([csd[0], csd[1], csd[2], csd[3]]),
    ]
}
