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

#define LS_USB3_PHY_ADDR 	PHYS_TO_UNCACHED(0x1f060000)

static void dev_fixup(void)
{
	u32 val;

	/* config all i2c as master */
	writeb(readb(LS_I2C0_REG_BASE + 2) | 0x20, LS_I2C0_REG_BASE + 2);
	writeb(readb(LS_I2C0_REG_BASE + 0x802) | 0x20, LS_I2C0_REG_BASE + 0x802);
	writeb(readb(LS_I2C0_REG_BASE + 0x1002) | 0x20, LS_I2C0_REG_BASE + 0x1002);
	writeb(readb(LS_I2C0_REG_BASE + 0x1802) | 0x20, LS_I2C0_REG_BASE + 0x1802);
	writeb(readb(LS_I2C0_REG_BASE + 0x2002) | 0x20, LS_I2C0_REG_BASE + 0x2002);
	writeb(readb(LS_I2C0_REG_BASE + 0x2802) | 0x20, LS_I2C0_REG_BASE + 0x2802);

	val = readl(LS_GENERAL_CFG0);
	val |= (1 << 9);	//hda pins use ac97
	writel(val, LS_GENERAL_CFG0);

	val = readl(LS_GENERAL_CFG1);
	val |= (1 << 1) | (1 << 28);	// usb0 as otg, lio 16bit
	writel(val, LS_GENERAL_CFG1);

	/*usb must init by the flow code otherwise cause error*/
	val = readl(LS_GENERAL_CFG1);
	val = (val & ~0x3fd) | 0x3e5;
	writel(val, LS_GENERAL_CFG1);

	/* SATA default setting */
	val = readl(PHYS_TO_UNCACHED(0x1fe10570));
	val = (val & ~(0xff << 16)) | (0x9f << 16);
	writel(val, PHYS_TO_UNCACHED(0x1fe10570));

	/* uart 0 and 1 be 2 line mode */
	val = readl(PHYS_TO_UNCACHED(0x1fe10100));
	val |= 0xee;
	writel(val, PHYS_TO_UNCACHED(0x1fe10100));
}

/*enable fpu regs*/
static void __maybe_unused fpu_enable(void)
{
	asm (
		"csrrd $r4, 0x2;\n\t"
		"ori   $r4, $r4, 1;\n\t"
		"csrwr $r4, 0x2;\n\t"
		:::"$r4"
	);
}

/*disable fpu regs*/
static void __maybe_unused fpu_disable(void)
{
	asm (
		"csrrd $r4, 0x2;\n\t"
		"ori   $r4, $r4, 1;\n\t"
		"xori  $r4, $r4, 1;\n\t"
		"csrwr $r4, 0x2;\n\t"
		:::"$r4"
	);
}

#ifdef CONFIG_SCSI_AHCI
static void ahci_setup(void)
{

}
#endif

#ifdef CONFIG_SCSI_AHCI_PLAT
void scsi_init(void)
{
	void __iomem *ahci_base;

	ahci_base = (void __iomem *)0x1f040000;
	printf("scsi ahci plat %p\n", ahci_base);
	if(!ahci_init(ahci_base))
		scsi_scan(1);
}
#endif

int board_early_init_f(void)
{
	// fpu_enable();
	dev_fixup();
#ifdef CONFIG_SCSI_AHCI
	ahci_setup();
#endif
	return 0;
}

#ifdef CONFIG_USB_XHCI_DWC3
static void xhci_setup(void)
{
	void __iomem *phy_base = (unsigned int *)LS_USB3_PHY_ADDR;

	u32 val = 0;
	u32 cfg = 0xff04;
	val = readl(phy_base + cfg);
	val |= (0x1 << 18);
	writel(val, phy_base + cfg);

	cfg = 0xc110;
	val = readl(phy_base + cfg);
	val |= (0x1 << 12);
	val &= (~(0x1 << 13));
	writel(val, phy_base + cfg);

	cfg = 0xff00;
	val = readl(phy_base + cfg);
	val &= (~(0x1 << 2));
	writel(val, phy_base + cfg);

	cfg = 0xc2c0;
	val = readl(phy_base + cfg);
	val &= (~(0x3 << 1));
	writel(val, phy_base + cfg);
}
#endif

static void regulator_init(void)
{
#ifdef CONFIG_DM_REGULATOR
	regulators_enable_boot_on(false);
#endif
}

#ifdef CONFIG_BOARD_EARLY_INIT_R
int board_early_init_r(void)
{
#ifdef CONFIG_USB_XHCI_DWC3
	/*init dwc3*/
	xhci_setup();
#endif

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

static void dpm_init(void)
{
	int ret;
	struct udevice *dev;
	ret = uclass_get_device_by_name(UCLASS_MISC,
						"dpm@1ff60000",
						&dev);
	if (ret < 0) {
		printf("Failed to find dpm node. Check device tree\n");
	}
}

#ifdef CONFIG_BOARD_LATE_INIT
int board_late_init(void)
{
	dpm_init();
	if (IS_ENABLED(CONFIG_LED)) {
		led_default_state();
	}

	return 0;
}
#endif

