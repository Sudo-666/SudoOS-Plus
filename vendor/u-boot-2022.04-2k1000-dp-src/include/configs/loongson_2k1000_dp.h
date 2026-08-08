/* SPDX-License-Identifier: GPL-2.0+ */
/*
 * EVB_LS2K500 configuration
 *
 * Copyright (c) 2022 Loongson Technologies
 * Author: Xiaochuan Mao<maoxiaochuan@loongson.cn>
 */

#ifndef __LOONGSON_LA_COMMON_H__
#define __LOONGSON_LA_COMMON_H__

#include <linux/sizes.h>
#include "loongson_common.h"

/* Loongson LS2K1000 clock configuration. */
#define REF_FREQ				100		// 参考时钟固定为100MHz
#define CORE_FREQ				900		// CPU 900Mhz@1.05v 1100Mhz@1.15v
#define DDR_FREQ				400		// MEM 400~600MHz
#define APB_FREQ				125		// APB 125MHz


/* Memory configuration */
#define CONFIG_SYS_SDRAM_BASE		(0x9000000000000000) /* cached address, use the low 256MB memory */
#define CONFIG_SYS_SDRAM_SIZE		(SZ_256M)
#define CONFIG_SYS_MONITOR_BASE		CONFIG_SYS_TEXT_BASE
#define CONFIG_SYS_BOOTPARAMS_LEN	SZ_64K

#ifdef CONFIG_SPL_BUILD
#define CONFIG_SPL_MAX_SIZE			SZ_256K
#define CONFIG_SPL_STACK			0x9000000090040000
#endif

/* UART configuration */
#define CONSOLE_BASE_ADDR			LS_UART0_REG_BASE
/* NS16550-ish UARTs */
#define CONFIG_SYS_NS16550_CLK		(APB_FREQ * 1000000)

#define CONFIG_SYS_CBSIZE	4096		/* Console I/O buffer size */
#define CONFIG_SYS_MAXARGS	32		/* Max number of command args */
#define CONFIG_SYS_BARGSIZE	CONFIG_SYS_CBSIZE 	/* Boot argument buffer size */


/* SATA configuration */
#if defined(CONFIG_SCSI_AHCI) && defined(CONFIG_PCI)
#define SCSI_VEND_ID 0x0014
#define SCSI_DEV_ID  0x7a08
#define CONFIG_SCSI_DEV_LIST {SCSI_VEND_ID, SCSI_DEV_ID}
#endif

/* LS SATA configuration */
#if defined(CONFIG_LOONGSON_AHCI)
#define CONFIG_LOONGSON_RECOVER
#endif

/* Miscellaneous configuration options */
#define CONFIG_SYS_BOOTM_LEN		(64 << 20)

/* Environment settings */
// #define CONFIG_ENV_SIZE			0x4000	/* 16KB */
#ifdef CONFIG_ENV_IS_IN_SPI_FLASH
// #define CONFIG_ENV_SIZE                 0x4000  /* 16KB */

/*
 * Environment is right behind U-Boot in flash. Make sure U-Boot
 * doesn't grow into the environment area.
 */
#define CONFIG_BOARD_SIZE_LIMIT         CONFIG_ENV_OFFSET
#endif

/* GMAC configuration */
#define CONFIG_DW_ALTDESCRIPTOR		// for designware ethernet driver.

/* OHCI configuration */
#ifdef CONFIG_USB_OHCI_HCD
#define CONFIG_USB_OHCI_NEW
#define CONFIG_SYS_USB_OHCI_MAX_ROOT_PORTS	1
#endif

/* video configuration */
// #define DISPLAY_BANNER_ON_VIDCONSOLE

/*print debug info in start.S*/
// #define DBG_ASM

#endif /* __LOONGSON_LA_COMMON_H__ */
