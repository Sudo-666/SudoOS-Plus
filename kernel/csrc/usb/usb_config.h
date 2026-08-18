#ifndef _USB_CONFIG_H
#define _USB_CONFIG_H

/*
 * SudoOS × LS2K1000 的 CherryUSB 宿主配置。
 *
 * 只启用: EHCI HCD + root-hub + MSC。关闭 CDC/HID/Audio/Video/外部 hub/
 * 中断式传输等一切无关功能。设计决策见 docs/decisions/ADR-001。
 */

/* 单 root port、无外部 hub。 */
#define CONFIG_USBHOST_RHPORTS 1
#define CONFIG_USBHOST_EHPORTS 1
#define CONFIG_USBHOST_INTF_NUM 4
#define CONFIG_USBHOST_EP_NUM 4
#define CONFIG_USBHOST_DEV_NAMELEN 16

/* 三个 host 线程的栈/优先级（M1 线程为桩，值暂未使用；M2 接调度器）。 */
#define CONFIG_USBHOST_PSC_PRIO 4
#define CONFIG_USBHOST_PSC_STACKSIZE 4096
#define CONFIG_USBHOST_HPWORKQ_PRIO 5
#define CONFIG_USBHOST_HPWORKQ_STACKSIZE 4096
#define CONFIG_USBHOST_LPWORKQ_PRIO 1
#define CONFIG_USBHOST_LPWORKQ_STACKSIZE 4096

#define CONFIG_USBHOST_ASYNCH

/*
 * CherryUSB 自带日志走 libc printf，由 build.rs 的 -DUSB_DBG_LEVEL=-1
 * 全部关掉（USB_DBG_ERROR=0 使 ERROR 级无条件启用，故用 -1 关闭所有级，
 * 且 usb_config.h 在 usb_util.h 之后被包含，无法在此覆盖）。M2 起如需
 * 真机枚举日志，改回 USB_DBG_INFO 并提供 mini printf。
 */

/*
 * LS2K1000 EHCI @ phys 0x4006_0000。
 *
 * 经 uncached DMW 窗口（0x8000_0000_0000_0000 | phys）直接访问 MMIO，
 * 与 LS2K1000 UART 驱动同款模式（arch/loongarch64/platform/ls2k1000）。
 * HCOR = HCCR + caplength，标准 EHCI caplength 为 0x10。
 */
#define CONFIG_USB_EHCI_HCCR_BASE (0x8000000040060000ULL)
#define CONFIG_USB_EHCI_HCOR_BASE (CONFIG_USB_EHCI_HCCR_BASE + 0x10)
#define CONFIG_USB_EHCI_QH_NUM 8
#define CONFIG_USB_EHCI_QTD_NUM 32

/*
 * M1 不启用 dcache 维护（CONFIG_USB_DCACHE_ENABLE /
 * CONFIG_USB_EHCI_DESC_DCACHE_ENABLE 均未定义）：
 * - 工具链 binutils 无法汇编 LoongArch `cache` 指令；
 * - uncached 描述符/缓冲方案需 linker section 手术，留到 M2 在真机确认
 *   DMA 一致性行为后再定（见 docs/decisions/ADR-001 的风险节）。
 * 因此 M1 阶段 EHCI 描述符/缓冲为 cached 内存，dcache 钩子为 no-op 宏。
 *
 * M2：LS2K1000 平台标记。EHCI 静态描述符池（QH/qTD/frame list）经
 * `.nocache_ram` 链接到 uncached DMW 窗口（见 linker.ld），
 * usb_ehci.c 据此：
 * - 给 DMA 全局加 section 属性；
 * - 用 `addr & PHYS_MASK` 覆盖恒等 physramaddr（缓存直接映射与 uncached
 *   窗口都是 `BASE | phys`）。
 */
#define CONFIG_USB_EHCI_LS2K1000

#endif
