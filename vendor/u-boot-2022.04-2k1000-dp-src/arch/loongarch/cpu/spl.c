#include <common.h>
#include <asm/global_data.h>
#include <spl.h>
#include <init.h>
#include <hang.h>

DECLARE_GLOBAL_DATA_PTR;

extern void ddr_init(void);
extern ulong spl_relocate_stack_gd(void);

__weak void spl_mach_init(void)
{
	arch_cpu_init();
}

__weak void spl_mach_init_late(void)
{
}

u32 spl_boot_device(void)
{
#if defined(CONFIG_SPL_SPI)
	return BOOT_DEVICE_SPI;
#elif defined(CONFIG_SPL_MMC)
	return BOOT_DEVICE_MMC1;
#elif defined(CONFIG_SPL_NAND_SUPPORT)
	return BOOT_DEVICE_NAND;
#else
	return BOOT_DEVICE_NONE;
#endif
}

__weak int spl_sdram_init(void)
{
	return dram_init();
}

void __noreturn board_init_f(ulong dummy)
{
	int ret;

	spl_mach_init();
	ret = spl_early_init();
	if (ret) {
		debug("spl_early_init() failed: %d\n", ret);
		hang();
	}

	if (spl_sdram_init()) {
		debug("spl sdram init failed\n");
		hang();
	}

	spl_mach_init_late();
	preloader_console_init();
	board_init_r(NULL, 0);
}
