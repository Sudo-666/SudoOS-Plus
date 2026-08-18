//! EHCI 控制器：probe / initialize / stop / poll。
//!
//! 初始化完全可破坏性安全地跑在 boot 期（scheduler 前）：adopt 路径跳过
//! HCRESET 保留 U-Boot 建立的 PHY/供电/连接（M2.8 教训——HCRESET 会清掉
//! PORTSC.PP），只重新编程调度列表地址/USBCMD/CONFIGFLAG；所有等待都有界
//! （busy-wait，≤ 100ms），绝不 spawn 线程。

use super::descriptor::DescPool;
use super::regs::{HcRegs, cap, cmd, intr, op, port, sts};
use super::root_hub;
use super::schedule::AsyncSchedule;
use crate::usb::dma::DmaPool;
use crate::usb::error::UsbError;
use crate::usb::platform::ls2k1000::{MAX_PORTS, busy_delay_ms, uncached_to_phys};

/// HCRESET / HCHalted 等待上限（ms）。
const HC_INIT_TIMEOUT_MS: u32 = 100;

/// EHCI 控制器（LS2K1000）。
pub struct EhciController {
    regs: HcRegs,
    /// HCSPARAMS 报告端口数（截断到 `MAX_PORTS`）。
    nports: usize,
    /// DMA 池（frame list + 描述符 + bounce）。
    pool: DmaPool,
    /// 描述符子分配器（QH/qTD 槽位，32B 对齐）。
    ///
    /// `dead_code`：RUSB-3 起用于 control/bulk QH 与 qTD 槽位分配。
    #[allow(dead_code)]
    desc_pool: DescPool,
    /// 异步调度（自环 async head QH）。
    schedule: AsyncSchedule,
}

impl EhciController {
    /// 构建控制器：非破坏——只切 DMA 槽位 + 建 frame list/async head。
    pub fn new(pool: DmaPool) -> Result<Self, UsbError> {
        let regs = HcRegs::ls2k1000();
        let nports = regs.nports().min(MAX_PORTS);
        let mut desc_pool = DescPool::new(pool.descriptors());
        let schedule = AsyncSchedule::init(pool.frame_list(), &mut desc_pool)?;
        Ok(Self {
            regs,
            nports,
            pool,
            desc_pool,
            schedule,
        })
    }

    /// 非破坏 MMIO 探测：能力/版本/端口数。
    pub fn probe(&self) {
        crate::println!(
            "RUSB-EHCI probe caplength={:02x} version={:04x} nports={} hccparams={:08x}",
            self.regs.caplength(),
            self.regs.hciversion(),
            self.nports,
            self.regs.read32(cap::HCCPARAMS),
        );
    }

    /// probe + initialize 一步完成（early_probe 调用）。
    pub fn probe_and_initialize(&mut self) {
        self.probe();
        match self.initialize() {
            Ok(()) => {}
            Err(error) => crate::println!("RUSB-EHCI initialize FAIL: {error:?}"),
        }
    }

    /// 控制器初始化：adopt-vs-reset + 调度地址 + RUN + CONFIGFLAG + PP 恢复
    /// + HCHalted 等待 + seed-attach。全 busy-wait，boot 期安全。
    pub fn initialize(&mut self) -> Result<(), UsbError> {
        let regs = self.regs;

        // adopt 判定：U-Boot 已初始化并停止（RUN=0、HALTED=1）、端口仍路由
        // 本控制器（CONFIGFLAG=1）、设备仍连接（PORTSC0.CCS=1）→ 复用
        // PHY/供电，跳过 HCRESET（C 驱动 usb_hc_init 2416-2429）。
        let cmd0 = regs.read32(op::USBCMD);
        let sts0 = regs.read32(op::USBSTS);
        let cf0 = regs.read32(op::CONFIGFLAG);
        let port0 = regs.read32(op::portsc(0));
        let adopt = cmd0 & cmd::RUN == 0
            && sts0 & sts::HALTED != 0
            && cf0 & 1 != 0
            && port0 & port::CCS != 0;

        if adopt {
            crate::println!("RUSB-EHCI handoff=uboot adopt");
        } else {
            crate::println!("RUSB-EHCI handoff=fresh hcreset");
            self.hc_reset()?;
        }

        // 禁中断 + 清状态（W1C 写 1 清）。
        regs.write32(op::USBINTR, 0);
        regs.write32(op::USBSTS, sts::W1C);

        // 列表地址：async head + frame list。
        regs.write32(op::CTRLDSSEGMENT, 0);
        regs.write32(
            op::PERIODICLISTBASE,
            uncached_to_phys(self.pool.frame_list().as_usize()),
        );
        regs.write32(op::ASYNCLISTADDR, self.schedule.head_physical());

        // USBCMD：ASEN + PSEN + FLSIZE_1024（先清 HCRESET/FLSIZE/PSEN/ASEN/IAADB）。
        let mut ucmd = regs.read32(op::USBCMD);
        ucmd &= !(cmd::HCRESET | cmd::FLSIZE_MASK | cmd::PSEN | cmd::ASEN | cmd::IAADB);
        ucmd |= cmd::ASEN | cmd::PSEN | cmd::FLSIZE_1024;
        regs.write32(op::USBCMD, ucmd);

        // RUN。
        ucmd = regs.read32(op::USBCMD) | cmd::RUN;
        regs.write32(op::USBCMD, ucmd);

        // 端口路由到本控制器。
        regs.write32(op::CONFIGFLAG, 1);

        // 恢复端口供电（adopt 路径 PP 已在，重申无害；fresh 路径复位后必
        // 须重设，否则端口断电——M2.8）。
        for p in 0..self.nports {
            root_hub::port_power(&regs, p);
        }

        // 等 HCHalted 清 0（控制器进入运行态）。
        self.wait_hc_running()?;

        // seed attach：adopt 路径已连接端口无 CSC 事件，主动记录（RUSB-4
        // 据此直接走 reset→枚举）。
        if adopt {
            for p in 0..self.nports {
                if root_hub::port_connected(&regs, p) {
                    crate::println!("RUSB-EHCI seeded-attach port={p} CCS=1");
                    break;
                }
            }
        }

        // 使能 EHCI 中断位（轮询模型；外部 IRQ 布线留待真机确认后）。
        regs.write32(op::USBINTR, intr::ALL);

        // 门禁：async 调度必须真正跑起来。
        let sts = regs.read32(op::USBSTS);
        let async_ok = sts & sts::ASS != 0;
        crate::println!(
            "RUSB-EHCI caps version={:04x} ports={} handoff={} async={} cmd={:08x} sts={:08x} {}",
            regs.hciversion(),
            self.nports,
            if adopt { "uboot" } else { "fresh" },
            if async_ok { "running" } else { "stopped" },
            regs.read32(op::USBCMD),
            sts,
            if async_ok { "PASS" } else { "FAIL" },
        );
        if !async_ok {
            return Err(UsbError::InvalidState);
        }
        Ok(())
    }

    /// HCRESET：置位后等自清。
    fn hc_reset(&self) -> Result<(), UsbError> {
        let regval = self.regs.read32(op::USBCMD) | cmd::HCRESET;
        self.regs.write32(op::USBCMD, regval);
        let start = crate::time::now();
        let wait = core::time::Duration::from_millis(HC_INIT_TIMEOUT_MS as u64);
        loop {
            if self.regs.read32(op::USBCMD) & cmd::HCRESET == 0 {
                return Ok(());
            }
            if crate::time::now().duration_since(start) >= wait {
                return Err(UsbError::Timeout);
            }
            busy_delay_ms(1);
        }
    }

    /// 等 `HCHalted` 清 0（RUN 后控制器进入运行态）。
    fn wait_hc_running(&self) -> Result<(), UsbError> {
        let start = crate::time::now();
        let wait = core::time::Duration::from_millis(HC_INIT_TIMEOUT_MS as u64);
        loop {
            if self.regs.read32(op::USBSTS) & sts::HALTED == 0 {
                return Ok(());
            }
            if crate::time::now().duration_since(start) >= wait {
                return Err(UsbError::Timeout);
            }
            busy_delay_ms(1);
        }
    }

    /// 停止控制器（RUN 清 0）。
    ///
    /// `dead_code`：RUSB-8 拆除 / 失败回退时使用。
    #[allow(dead_code)]
    pub fn stop(&mut self) {
        let cmd = self.regs.read32(op::USBCMD) & !cmd::RUN;
        self.regs.write32(op::USBCMD, cmd);
    }

    /// 轮询：清 USBSTS W1C 位。
    ///
    /// `dead_code`：RUSB-4 端口变化处理（CSC/PEC）时启用。
    #[allow(dead_code)]
    pub fn poll(&mut self) {
        let sts = self.regs.read32(op::USBSTS);
        if sts & sts::W1C != 0 {
            self.regs.write32(op::USBSTS, sts & sts::W1C);
        }
    }
}
