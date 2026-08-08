#include <common.h>
#include <init.h>
#include <spl.h>
#include <asm/addrspace.h>
#include <asm/sections.h>
#include <asm/io.h>
#include <asm/global_data.h>
#include <spi.h>
#include <spi_flash.h>
#include <mach/loongson.h>
#include <image.h>

DECLARE_GLOBAL_DATA_PTR;

void board_boot_order(u32 *spl_boot_list)
{
    int n = 0;

#ifdef CONFIG_SPL_SPI
    spl_boot_list[n++] = BOOT_DEVICE_SPI;
#endif
#ifdef CONFIG_SPL_MMC
	spl_boot_list[n++] = BOOT_DEVICE_MMC1;
#endif
#ifdef CONFIG_SPL_NAND_SUPPORT
    spl_boot_list[n++] = BOOT_DEVICE_NAND;
#endif
}

static unsigned int spl_get_uboot_offs(void)
{
	ulong spl_size, fdt_size;
    unsigned int offs;

    if (fdt_check_header(gd->fdt_blob)) {
#ifdef CONFIG_SYS_SPI_U_BOOT_OFFS
		return CONFIG_SYS_SPI_U_BOOT_OFFS;
#else
        return 0;
#endif
	}

    fdt_size = fdt_totalsize(gd->fdt_blob);
    spl_size = (ulong)&_image_binary_end - (ulong)__text_start;

    offs = (unsigned int)(spl_size + fdt_size);
    return offs;
}

#ifdef CONFIG_SPL_SPI_LOAD
unsigned int spl_spi_get_uboot_offs(struct spi_flash *flash)
{
    return spl_get_uboot_offs();
}
#endif

void spl_board_prepare_for_boot(void)
{

}

#ifdef CONFIG_SPL_DISPLAY_PRINT
void spl_display_print(void)
{
    // ascii art (style: Matchsticks)
    printf("\n"
        " _     __   __  _  _  ___  ___  __  _  _    /   ___  __  \\ \n"
        " |    |  | |  | |\\ | | __ [__  |  | |\\ |    |  | __ |  \\ | \n"
        " |___ |__| |__| | \\| |__] ___] |__| | \\|    \\  |__] |__/ / \n"
        "\n");
}
#endif
