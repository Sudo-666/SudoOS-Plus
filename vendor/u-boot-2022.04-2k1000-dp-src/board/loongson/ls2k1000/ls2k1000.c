// SPDX-License-Identifier: GPL-2.0+

#include <common.h>
#include <env.h>
#include <pci.h>
#include <usb.h>
#include <scsi.h>
#include <ahci.h>
#include <led.h>
#include <asm/io.h>
#include <dm.h>
#include <mach/loongson.h>
#include <power/regulator.h>

extern void update_slave_core(void);

static void mem_win_cfg(void)
{
	/* CPU_WIN0 */
	writeq(0x1c000000, LS_CPU_WIN0_BASE);
	writeq(0xfffffffffff00000, LS_CPU_WIN0_MASK);
	// writeq(0x1fc000f2, LS_CPU_WIN0_MMAP);
	// disable instruct fetch for spi io mode
	writeq(0x1fc00082, LS_CPU_WIN0_MMAP);

	/* CPU_WIN1 */
	writeq(0x10000000, LS_CPU_WIN1_BASE);
	writeq(0xfffffffff0000000, LS_CPU_WIN1_MASK);
	writeq(0x10000082, LS_CPU_WIN1_MMAP);

	/* CPU_WIN2 */
	writeq(0x0, LS_CPU_WIN2_BASE);
	writeq(0xfffffffff0000000, LS_CPU_WIN2_MASK);
	writeq(0xf0, LS_CPU_WIN2_MMAP);

	/* CPU_WIN3 */
	writeq(0x80000000, LS_CPU_WIN3_BASE);
	writeq(0xffffffff80000000, LS_CPU_WIN3_MASK);
	writeq(0xf0, LS_CPU_WIN3_MMAP);

	/* CPU_WIN4 */
	writeq(0x100000000, LS_CPU_WIN4_BASE);
	writeq(0xffffffff00000000, LS_CPU_WIN4_MASK);
	writeq(0x1000000f0, LS_CPU_WIN4_MMAP);


	/* CPU_WIN5 */
	writeq(0x200000000, LS_CPU_WIN5_BASE);
	writeq(0xffffffff80000000, LS_CPU_WIN5_MASK);
	writeq(0x800000f0, LS_CPU_WIN5_MMAP);

	/* CPU_WIN6 */
	writeq(0x0, LS_CPU_WIN6_BASE);
	writeq(0x0, LS_CPU_WIN6_MASK);
	writeq(0x0, LS_CPU_WIN6_MMAP);

	/* CPU_WIN7 */
	writeq(0x0, LS_CPU_WIN7_BASE);
	writeq(0x0, LS_CPU_WIN7_MASK);
	writeq(0x0, LS_CPU_WIN7_MMAP);
}

static void dev_fixup(void)
{
	/* Disable USB prefetch */
	writel(readl(LS_GENERAL_CFG1) & ~(1 << 19), LS_GENERAL_CFG1);
}

static void usb_phy_config_single_ls2k1000_LA_JL(unsigned int addr)
{
	unsigned long value;
	value = readq(PHYS_TO_UNCACHED(addr));

	// -4 ohm (ori 1.5 ohm)
	value |= ((unsigned long)3) << 57;
	value |= ((unsigned long)3) << 25;
	// +15mv about dp dm (ori 0mv)
	value |= ((unsigned long)2) << 52;
	value |= ((unsigned long)2) << 20;
	// +0% impedance ori (ori +5%)
	value |= ((unsigned long)3) << 42;
	value |= ((unsigned long)3) << 10;

	// +0% vol about check outlink (ori -6%)
	value |= ((unsigned long)4) << 36;
	value |= ((unsigned long)4) << 4;
	writeq(value, PHYS_TO_UNCACHED(addr));

	//enable sofeware set usb phy param
	value |= ((unsigned long)1) << 32;
	value |= ((unsigned long)1) << 0;
	writeq(value, PHYS_TO_UNCACHED(addr));
}

static void usb_phy_config_ls2k1000_LA_JL(void)
{
	usb_phy_config_single_ls2k1000_LA_JL(0x1fe00440);
	usb_phy_config_single_ls2k1000_LA_JL(0x1fe00448);
}

typedef void (*usb_phy_config_func)(void);

static usb_phy_config_func usb_phy_config_func_set[10] = {NULL, usb_phy_config_ls2k1000_LA_JL,};

static void usb_phy_config(void)
{
	int node;
	int type;

	node = fdt_path_offset(gd->fdt_blob, "/soc");
	if (node < 0)
		return;

	type = 0;
	fdtdec_get_int_array(gd->fdt_blob, node, "usb_config_type", &type, 1);
	if(!type)
		return;

	(*usb_phy_config_func_set[type])();
}

int board_early_init_f(void)
{
	mem_win_cfg();
	dev_fixup();
	return 0;
}

static void regulator_init(void)
{
#ifdef CONFIG_DM_REGULATOR
	regulators_enable_boot_on(false);
#endif
}

#ifdef CONFIG_BOARD_EARLY_INIT_R
int board_early_init_r(void)
{
	// sync core1 to run in ram.
	update_slave_core();

#ifdef CONFIG_DM_PCI
	/*
	 * Make sure PCI bus is enumerated so that peripherals on the PCI bus
	 * can be discovered by their drivers
	 */
	pci_init();
#endif

	regulator_init();

	return 0;
}
#endif

#ifdef CONFIG_BOARD_LATE_INIT
int board_late_init(void)
{
	if (IS_ENABLED(CONFIG_LED)) {
		led_default_state();
	}

	return 0;
}
#endif

#ifdef CONFIG_SPL_BOARD_INIT
void spl_board_init(void)
{
	mem_win_cfg();
	usb_phy_config();
}
#endif
