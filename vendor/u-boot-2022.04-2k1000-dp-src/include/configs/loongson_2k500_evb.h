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

/* Loongson LS2K500 clock configuration. */
#define REF_FREQ				100		//参考时钟固定为100MHz
#define CORE_FREQ				500		//CPU 500Mhz
#define DDR_FREQ				300		//MEM 400~600Mhz
#define SB_FREQ					100		//SB 100~200MHz, for BOOT, SATA, USB, APB
#define NET_FREQ				200		//NETWORK 200~400MHz, for NETWORK, DC, PRINT


/* Memory configuration */
// #define CONFIG_SYS_BOOTPARAMS_LEN	SZ_64K
#define CONFIG_SYS_SDRAM_BASE		(0x9000000000000000) /* cached address, use the low 256MB memory */
#define CONFIG_SYS_SDRAM_SIZE		(SZ_256M)
#define CONFIG_SYS_MONITOR_BASE		CONFIG_SYS_TEXT_BASE

#ifdef CONFIG_SPL_BUILD
#define CONFIG_SPL_MAX_SIZE			SZ_256K
#define CONFIG_SPL_STACK			0x9000000090040000
#endif

/* UART configuration */
#define CONSOLE_BASE_ADDR			LS_UART2_REG_BASE
/* NS16550-ish UARTs */
#define CONFIG_SYS_NS16550_CLK		(SB_FREQ * 1000000)	// CLK_in: 100MHz

#define CONFIG_SYS_CBSIZE	4096		/* Console I/O buffer size */
#define CONFIG_SYS_MAXARGS	32		/* Max number of command args */
#define CONFIG_SYS_BARGSIZE	CONFIG_SYS_CBSIZE 	/* Boot argument buffer size */

/* NAND configuration */
#ifdef CONFIG_MTD_RAW_NAND
#define CONFIG_SYS_MAX_NAND_DEVICE 4
#define CONFIG_NAND_ECC_BCH
#endif

/* SATA configuration */
#if defined(CONFIG_SCSI_AHCI) && defined(CONFIG_PCI)
#define SCSI_VEND_ID 0x0014
#define SCSI_DEV_ID  0x7a08
#define CONFIG_SCSI_DEV_LIST {SCSI_VEND_ID, SCSI_DEV_ID}
#endif

/* Miscellaneous configuration options */
#define CONFIG_SYS_BOOTM_LEN		(64 << 20)

/* Environment settings */
// #define CONFIG_ENV_SIZE			0x4000	/* 16KB */
#ifdef CONFIG_ENV_IS_IN_SPI_FLASH

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

#endif /* __LOONGSON_LA_COMMON_H__ */
