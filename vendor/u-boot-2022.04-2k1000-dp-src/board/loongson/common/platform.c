// SPDX-License-Identifier: GPL-2.0+

#include <common.h>
#include <command.h>
#include <linux/sizes.h>
#include <asm/global_data.h>
#include <asm/sections.h>
#include <asm/addrspace.h>
#include <dm.h>
#include <mapmem.h>
#include <video_console.h>
#include <usb.h>
#include <mach/loongson.h>
#include <nand.h>
#include <env.h>

#define NOTICE_STR1 "c to enter u-boot console"
#define NOTICE_STR2 "m to enter boot menu"

#define ANSI_CURSOR_SAVE		"\e7"
#define ANSI_CURSOR_RESTORE		"\e8"

DECLARE_GLOBAL_DATA_PTR;

ulong board_get_usable_ram_top(ulong total_size)
{
	phys_addr_t ram_top = gd->ram_top;

	// U-boot 阶段仅使用最低的 256MB 内存，以便兼容不同内存容量的板卡。
	// 0x8F00_0000 ~ 0x8FFF_FFFF (16MB) 保留, 用于固件与内核的信息交互, 
	// 具体参考 《龙芯CPU开发系统固件与内核接口详细规范》。
	// 注意：DVO0, DVO1 framebuffer 的保留内存 32MB (0x0D00_0000 ~ 0x0F00_0000),
	// 将在 board_f.c reserve_video() 中保留，此处无需处理。
	if (VA_TO_PHYS(ram_top) >= (phys_addr_t)(MEM_WIN_BASE + SZ_256M - SZ_16M)) {
		ram_top = (phys_addr_t)map_sysmem(MEM_WIN_BASE + SZ_256M - SZ_16M, 0);
	}

#ifdef CONFIG_SPL
	// Keep 4MB space for u-boot image.
	ram_top -= SZ_4M;
	if (CONFIG_SYS_TEXT_BASE < ram_top) {
		printf("Warning: Run u-boot from SPL, "
				"but the CONFIG_SYS_TEXT_BASE is out of "
				"the reserved space for u-boot image\n");
	}
#endif

	return ram_top;
}

#ifdef CONFIG_OF_BOARD
void *board_fdt_blob_setup(int *err)
{
#ifdef CONFIG_SPL
	uint8_t *fdt_dst;
	uint8_t *fdt_src;
	ulong size;

	*err = 0;
	fdt_dst = (uint8_t*)ALIGN((ulong)&__bss_end, ARCH_DMA_MINALIGN);
#ifdef CONFIG_SPL_BUILD
	fdt_src = (uint8_t*)((ulong)&_image_binary_end - (ulong)__text_start +
                        BOOT_SPACE_BASE_UNCACHED);
#else
	fdt_src = (uint8_t*)&_image_binary_end;
	//当fdt段落在bss段里面时，需要把fdt复制到bss外面，因为启动过程中bss段会被清零，
	//当fdt段落在bss段外面时，返回fdt段的地址即可。
	if (((ulong)fdt_src > (ulong)&__bss_start) && ((ulong)fdt_src < (ulong)&__bss_end)) {

	} else {
		fdt_dst = fdt_src;
	}
#endif

	if ((fdt_dst != fdt_src) && 
			(fdt_magic(fdt_src) == FDT_MAGIC)) {
		size = ALIGN(fdt_totalsize(fdt_src), 32);
		if ((fdt_src < fdt_dst && ((ulong)fdt_src + size) > (ulong)fdt_dst) ||
				(fdt_dst < fdt_src && ((ulong)fdt_dst + size) > (ulong)fdt_src)) {
			printf("!!fdt_blob space overlap!!\n");
			return (void*)gd->fdt_blob;
		}

		memcpy(fdt_dst, fdt_src, size);
		gd->fdt_size = size;
	}
	return fdt_dst;
#else
	*err = 0;
	return (void*)gd->fdt_blob;
#endif
}
#endif

static void acpi_config(void)
{
	volatile u32 val;
	// disable wake on lan
	val = readl(LS_PM_RTC_REG);
	writel(val & ~(0x3 << 7), LS_PM_RTC_REG);

	// disable pcie and rtc wakeup
	val = readl(LS_PM1_EN_REG);
	val &= ~(1 << 10);
	val |= 1 << 14;
	writel(val, LS_PM1_EN_REG);

	// disable usb/ri/gmac/bat/lid wakeup event
	// and enable cpu thermal interrupt.
	writel(0xe, LS_GPE0_EN_REG);

	// clear pm status
	writel(0xffffffff, LS_PM1_STS_REG);
}

#ifdef CONFIG_MISC_INIT_R
int misc_init_r(void)
{
	acpi_config();
	return 0;
}
#endif

/*
 * display uboot version info on lcd
 * if you make with -DDISPLAY_BANNER_ON_VIDCONSOLE it will
 * display it on lcd
 */
static int vidconsole_notice(char *buf)
{
#ifdef CONFIG_DM_VIDEO
	struct vidconsole_priv *priv;
	struct udevice *con, *vdev = NULL;
	int col, row, len, ret;
	int vidcon_id = 0;

	for ( ret = uclass_first_device(UCLASS_VIDEO, &vdev);
				vdev; uclass_next_device(&vdev)) {
		debug("video device: %s\n", vdev->name);
		vidcon_id++;
	}

	if (vidcon_id < 1) {
		debug("vidconsole no exist\n");
		sprintf(buf, "Press %s, %s", NOTICE_STR1, NOTICE_STR2);
		return 0;
	}

	for (ret = uclass_first_device(UCLASS_VIDEO_CONSOLE, &con);
				con; uclass_next_device(&con)) {
		vidconsole_put_string(con, ANSI_CURSOR_SAVE);
		priv = dev_get_uclass_priv(con);
		row = priv->rows - 1;
#ifdef DISPLAY_BANNER_ON_VIDCONSOLE
		display_options_get_banner(false, buf, DISPLAY_OPTIONS_BANNER_LENGTH);
		len = strcspn(buf, "\n");
		buf[len] = 0;
		col = (priv->cols / 2) - (len / 2);
		if (col < 0)
			col = 0;
		vidconsole_position_cursor(con, col, row);
		vidconsole_put_string(con, buf);
		row -= 1;
#endif
		sprintf(buf, "Press %s, %s", NOTICE_STR1, NOTICE_STR2);
		len = strlen(buf);
		col = (priv->cols / 2) - (len / 2);
		if (col < 0)
			col = 0;
		vidconsole_position_cursor(con, col, row);
		vidconsole_put_string(con, buf);
		vidconsole_position_cursor(con, 0, 0);
		vidconsole_put_string(con, ANSI_CURSOR_RESTORE);
	}
#else
	sprintf(buf, "Press %s, %s", NOTICE_STR1, NOTICE_STR2);
#endif
	return 0;
}

static void print_notice(void)
{
	char notice[DISPLAY_OPTIONS_BANNER_LENGTH];

	if (vidconsole_notice(notice))
		sprintf(notice, "Press %s, %s", NOTICE_STR1, NOTICE_STR2);

	printf("************************** Notice **************************\n");
	printf("%s\r\n", notice);
	printf("************************************************************\n");
}

static void loongson_env_trigger(void)
{
	run_command("loongson_env_trigger init", 0);
	run_command("loongson_env_trigger ls_trigger_u_kernel", 0);
	run_command("loongson_env_trigger ls_trigger_u_rootfs", 0);
	run_command("loongson_env_trigger ls_trigger_u_uboot", 0);
	run_command("loongson_env_trigger ls_trigger_boot", 0);
}

#ifdef CONFIG_LAST_STAGE_INIT
// extern void user_env_fetch(void);
extern int recover(void);

#ifdef CONFIG_MTD_RAW_NAND
void adjust_nand_pagesize(void)
{
	struct mtd_info* mtd;
	mtd = get_nand_dev_by_index(0);
	if (mtd) {
		int cur_page_size;
		char cur_page_size_str[10];
		char *env_page_size_str;
		char *default_page_size_str = "2048";

		env_page_size_str = env_get("nand_pagesize");
		if (!env_page_size_str) {
			printf("nand_pagesize is null, use default pagesize(2048)\n");
			env_page_size_str = default_page_size_str;
			env_set("nand_pagesize", env_page_size_str);
		}

		cur_page_size = mtd->writesize;

		memset(cur_page_size_str, 0, 10);
		sprintf(cur_page_size_str, "%d", cur_page_size);
		cur_page_size_str[9] = 0;

		if (strcmp(cur_page_size_str, env_page_size_str)) {
			printf("change env nand_pagesize to be %s(ori is %s)\n", cur_page_size_str, env_page_size_str);
			env_set("nand_pagesize", cur_page_size_str);
		}
	}
}
#endif

int last_stage_init(void)
{
	print_notice();
	// user_env_fetch();

#ifdef CONFIG_USB_KEYBOARD
	/* start usb so that usb keyboard can be used as input device */
	usb_init();
#endif

#ifdef CONFIG_LOONGSON_RECOVER
	/* 上电时长按按钮3秒进入recover功能, recover优先顺序usb>mmc>sata
       按键使用 LS_RECOVER_GPIO_BUTTON 定义的gpio */
	recover();
#endif

#ifdef CONFIG_MTD_RAW_NAND
	adjust_nand_pagesize();
#endif

	loongson_env_trigger();

	return 0;
}
#endif

int checkboard(void)
{
	struct udevice *dev;
	ofnode parent_node, node;
	const char *bdname;

	uclass_first_device(UCLASS_SYSINFO, &dev);
	if (dev) {
		parent_node = dev_read_subnode(dev, "smbios");
		if (!ofnode_valid(parent_node))
			goto out;

		node = ofnode_find_subnode(parent_node, "baseboard");
		if (!ofnode_valid(node))
			goto out;

		bdname = ofnode_read_string(node, "product");
		if (bdname)
			printf("Board: %s\n", bdname);
	}

out:
	return 0;
}

#if defined(CONFIG_SYS_CONSOLE_IS_IN_ENV) && \
	defined(CONFIG_SYS_CONSOLE_OVERWRITE_ROUTINE)
int overwrite_console(void)
{
	return 0;
}
#endif

#ifdef CONFIG_SPL_BOARD_INIT
__weak void spl_board_init(void)
{
	return ;
}
#endif

// Please use the scsi with driver model(CONFIG_DM_SCSI) first!
#ifdef CONFIG_SCSI_AHCI_PLAT
void scsi_init(void)
{
	void __iomem *ahci_base;

	ahci_base = (void __iomem *)LS_SATA_BASE;
	printf("scsi ahci plat %p\n", ahci_base);
	if(!ahci_init(ahci_base))
		scsi_scan(1);
}
#endif
