// SPDX-License-Identifier: GPL-2.0+

#include <common.h>
#include <command.h>
#include <malloc.h>
#include <asm/cache.h>
#include <asm/global_data.h>

#include <jffs2/jffs2.h>

#include "loongson_update.h"

extern int mtdparts_init(void);

DECLARE_GLOBAL_DATA_PTR;

const char *update_devname_str[UPDATE_DEV_COUNT] = {
	[UPDATE_DEV_USB]	= "usb",
	[UPDATE_DEV_TFTP]	= "tftp",
	[UPDATE_DEV_MMC]	= "mmc",
	[UPDATE_DEV_DHCP]	= "dhcp",
};

const char *update_typename_str[UPDATE_TYPE_COUNT] = {
	[UPDATE_TYPE_KERNEL]	= "kernel",
	[UPDATE_TYPE_ROOTFS]	= "rootfs",
	[UPDATE_TYPE_UBOOT]		= "uboot",
	[UPDATE_TYPE_DTB]		= "dtb",
	[UPDATE_TYPE_SYSTEM]	= "system",
	[UPDATE_TYPE_ALL]		= "all",
	[UPDATE_TYPE_BOOTSELECT]		= "bootselect",
	[UPDATE_TYPE_RESOLUTION]		= "resolution",
};

static int mtdparts_probe(void)
{
	int ret = -1;

	run_command("sf probe", 0);

	ret = mtdparts_init();
	if (ret) {
		printf("-----> mtdparts init failed! partition not found or toolarge\r\n");
	}
	return ret;
}

static void user_env_save(void)
{
	char *env;
	char *mem;
	char *cmd;
#define ENV_BUF_LEN 1024

	printf("save bdinfo environment\r\n");

	env = env_get("syspart");//syspart 用于保存操作系统分区的分区号(linux 如sda1、sda2 ...) (uboot 如scsi 0:1、scsi 0:2 ...)
	if (env == NULL)
		return;

	mem = (char *)(memalign(ARCH_DMA_MINALIGN, ENV_BUF_LEN));
	cmd = (char *)(memalign(ARCH_DMA_MINALIGN, 128));
	memset(mem, 0, ENV_BUF_LEN);
	memset(mem, 0, 128);

	sprintf(mem, "syspart=%s", env);

#ifdef LS_DOUBLE_SYSTEM
	env = env_get("syspart_last");
	sprintf(mem + strlen(mem) + 1, "syspart_last=%s", env ? env : env_get("syspart"));
#endif

	sprintf(cmd, "mtd erase bdinfo");
	run_command(cmd, 0);
	sprintf(cmd, "mtd write bdinfo 0x%p 0 0x%x", mem, ENV_BUF_LEN);
	run_command(cmd, 0);

	free(cmd);
	free(mem);
}

void user_env_fetch(void)
{
	char *env;
	char *mem;
	char *cmd;
	char *env_addr;
	char *env_user;	
	int ret;
#define ENV_BUF_LEN 1024

#ifdef LS_DOUBLE_SYSTEM
#else
	env = env_get("update"); //update 可用于判断环境变量是否被更新过，如spi flash的环境变量分区是否被擦除；
							 //也可用于判断uboot是否被更新过。update默认值是1，uboot第一次启动时可以把该变量修改为0，
							 //下次启动就不会再读取spi_user的用户分区
	if (strcmp(env, "0") == 0) {
		return;
	}
#endif

	printf("%s\r\n", __func__);

	env_user = (char *)(memalign(ARCH_DMA_MINALIGN, ENV_BUF_LEN));
	mem = (char *)(memalign(ARCH_DMA_MINALIGN, ENV_BUF_LEN));
	cmd = (char *)(memalign(ARCH_DMA_MINALIGN, 128));

	sprintf(cmd, "sf probe;mtd read bdinfo 0x%p 0 0x%x", mem, ENV_BUF_LEN);
	run_command(cmd, 0);

	env_addr = (char *)gd->env_addr;
	gd->env_addr = (ulong)mem;
	ret = env_get_f("syspart", env_user, ENV_BUF_LEN);
	gd->env_addr = (ulong)env_addr;

#ifdef LS_DOUBLE_SYSTEM
	env = env_get("update"); //update 可用于判断环境变量是否被更新过，如spi flash的环境变量分区是否被擦除；
							 //也可用于判断uboot是否被更新过。update默认值是1，uboot第一次启动时可以把该变量修改为0，
							 //下次启动就不会再读取spi_user的用户分区
	if (strcmp(env, "0") != 0) {
		if (ret > 0) {
			env = env_get("syspart");
			if ((strcmp(env_user, env) != 0) || (env_get("syspart_last") == NULL)) {
				env_set("syspart", env_user);

				env_addr = (char *)gd->env_addr;
				gd->env_addr = (ulong)mem;
				ret = env_get_f("syspart_last", env_user, ENV_BUF_LEN);
				gd->env_addr = (ulong)env_addr;

				env_set("syspart_last", env_user ? env_user : env_get("syspart"));
			}
		}
		env_set("update", "0");
		env_save();
	} else {
		if (ret > 0) {
			env = env_get("syspart");
			if (strcmp(env_user, env) != 0) {
				env_set("syspart_last", env_user);
				env_save();
				user_env_save();
			}
		}
	}
#else
	if (ret > 0) {
		env = env_get("syspart");
		if (strcmp(env_user, env) != 0) {
			env_set("syspart", env_user);
		}
	}

	env_set("update", "0");
	env_save();
#endif

	free(cmd);
	free(mem);
	free(env_user);
}

static void update_failed_way_tip(int way)
{
	if (way == UPDATE_DEV_USB) {
		printf("### check usb filesystem type must be fat32\n");
	} else if (way == UPDATE_DEV_TFTP) {
		printf("### check tftp server running in PC\n");
		printf("### check board link PC by eth cable(RJ45)\n");
		printf("### check uboot env ipaddr and serverip(PC ip)\n");
	} else if (way == UPDATE_DEV_MMC) {
		printf("### check sd card filesystem type must be fat32\n");
	} else if (way == UPDATE_DEV_DHCP) {
		printf("### check tftp server running in PC\n");
		printf("### check board link PC by eth cable(RJ45)\n");
		printf("### check uboot env serverip(PC ip)\n");
		printf("### check board and PC link router(not PC link board)\n");
	}
}

static void update_failed_target_tip(int way, int target)
{
	char uboot_file[] = "u-boot-with-spl.bin";
	char kernel_file[] = "uImage";
	char rootfs_file[] = "rootfs-ubifs-ze.img";
	char dtb_file[] = "dtb.bin";
	char* file;

	if (target == UPDATE_TYPE_KERNEL) {
		printf("### check where update kernel(nand? sata?)\n");
		file = kernel_file;
	}
	else if (target == UPDATE_TYPE_ROOTFS)
		file = rootfs_file;
	else if (target == UPDATE_TYPE_UBOOT)
		file = uboot_file;
	else if (target == UPDATE_TYPE_DTB)
		file = dtb_file;

	if (way == UPDATE_DEV_USB) {
		printf("### ensure %s in update dir(usb)\n", file);
	} else if (way == UPDATE_DEV_TFTP) {
		printf("### ensure %s in tftp server dir\n", file);
	} else if (way == UPDATE_DEV_MMC) {
		printf("### ensure %s in update dir(sd card)\n", file);
	} else if (way == UPDATE_DEV_DHCP) {
		printf("### ensure %s in tftp server dir\n", file);
	}
}

static void update_result_display(int res, int way, int target)
{
	printf("\n");

	printf("######################################################\n");
	printf("### update target: %s\n", update_typename_str[target]);
	printf("### update way   : %s\n", update_devname_str[way]);
	printf("### update result: %s\n", res ? "failed" : "success");


	if (res) {
		printf("###\n");
		printf("### tip:\n");
		update_failed_way_tip(target);
		update_failed_target_tip(way, target);
	}

	printf("######################################################\n\n");
}

static int update_uboot(int dev)
{
	int ret = -1;
	char cmd[256];
	char *image_name[] = {
		"u-boot-with-spl.bin",
		"u-boot.bin"
	};

	printf("update u-boot.............\n");

	for (int i = 0; i < sizeof(image_name)/sizeof(image_name[0]); i++) {
		printf("try to get %s ......\n", image_name[i]);
		switch (dev) {
		case UPDATE_DEV_USB:
			sprintf(cmd, "fatload usb 0:1 ${loadaddr} /update/%s", image_name[i]);
			break;
		case UPDATE_DEV_TFTP:
			sprintf(cmd, "tftpboot ${loadaddr} %s", image_name[i]);
			break;
		case UPDATE_DEV_MMC:
			sprintf(cmd, "fatload mmc 0:1 ${loadaddr} /update/%s", image_name[i]);
			break;
		case UPDATE_DEV_DHCP:
			sprintf(cmd, "dhcp ${loadaddr} ${serverip}:%s", image_name[i]);
			break;
		}

		ret = run_command(cmd, 0);
		if (!ret)
			break;
	}

	if (ret) {
		if (dev == UPDATE_DEV_USB) {
			printf("upgrade uboot failed, try again now!\r\n");
			for (int i = 0; i < sizeof(image_name)/sizeof(image_name[0]); i++) {
				printf("try to get %s ......\n", image_name[i]);
				sprintf(cmd, "fatload usb 0:0 ${loadaddr} /update/%s", image_name[i]);
				ret = run_command(cmd, 0);
				if (!ret)
					break;
			}
		}
		if (ret) {
			printf("upgrade uboot failed, not found u-boot image!\r\n");
			return ret;
		}
	}

	if (mtdparts_probe()) {
		ret = run_command("sf erase 0 0xd8000;sf update ${loadaddr} 0 ${filesize}", 0);
	} else {
		struct mtd_device *mtd_dev;
		struct part_info *part;
		u8 pnum;
		ulong uboot_size, env_size;

		find_dev_and_part("uboot", &mtd_dev, &pnum, &part);
		uboot_size = part->size;
		find_dev_and_part("uboot_env", &mtd_dev, &pnum, &part);
		env_size = part->size;

		// erase operation maybe take long time, show the informations.
		printf("Erase uboot partition ... ");
		sprintf(cmd, "sf erase uboot %lx", uboot_size);
		ret = run_command(cmd, 0);
		if (ret)
			goto out;

		// sprintf(cmd, "sf erase uboot_env %lx", env_size);
		// ret = run_command(cmd, 0);
		// if (ret)
		// 	goto out;

		sprintf(cmd, "sf update ${loadaddr} uboot ${filesize}");
		ret = run_command(cmd, 0);
		if (ret)
			goto out;
	}

	user_env_save();

out:
	update_result_display(ret, dev, UPDATE_TYPE_UBOOT);

	return ret;
}

static int update_dtb(int dev)
{
	int ret = -1;

	printf("update dtb.............\n");

	switch (dev) {
	case UPDATE_DEV_USB:
		ret = run_command("fatload usb 0:1 ${loadaddr} /update/dtb.bin", 0);
		break;
	case UPDATE_DEV_TFTP:
		ret = run_command("tftpboot ${loadaddr} dtb.bin", 0);
		break;
	case UPDATE_DEV_MMC:
		ret = run_command("fatload mmc 0:1 ${loadaddr} /update/dtb.bin", 0);
		break;
	}

	if (ret) {
		if (dev == UPDATE_DEV_USB) {
			printf("upgrade dtb failed, try again now!\r\n");
			ret = run_command("fatload usb 0:0 ${loadaddr} /update/dtb.bin", 0);
		}
		if (ret) {
			printf("upgrade dtb failed, not found /update/dtb.bin file!\r\n");
			return ret;
		}
	}

	if (mtdparts_probe()) {
		ret = -1;
	} else {
		ret = run_command("sf erase dtb 0x20000;sf update ${loadaddr} dtb ${filesize}", 0);
	}

	update_result_display(ret, dev, UPDATE_TYPE_DTB);

	return ret;
}

static int update_kernel(int dev, char *typename, char *kernelname)
{
	char cmd[256];
	char kernel[32] = "uImage";
	int ret = -1;

	printf("update kernel....................\n");

	if (!typename) {
		printf("error! not appoint where update kernel(nand? sata?)\n");
		goto err;
	}

	if (kernelname) {
		memset(kernel, 0, sizeof(kernel));
		sprintf(kernel, "%s", kernelname);
	}

	switch (dev) {
	case UPDATE_DEV_USB:
		sprintf(cmd, "fatload usb 0:1 ${loadaddr} update/%s", kernel);
		break;
	case UPDATE_DEV_TFTP:
		sprintf(cmd, "tftpboot ${loadaddr} %s", kernel);
		break;
	case UPDATE_DEV_MMC:
		sprintf(cmd, "fatload mmc 0:1 ${loadaddr} update/%s", kernel);
	case UPDATE_DEV_DHCP:
		sprintf(cmd, "dhcp ${loadaddr} ${serverip}:%s", kernel);
		break;
	}

	ret = run_command(cmd, 0);
	if (ret) {
		if (dev == UPDATE_DEV_USB) {
			printf("upgrade kernel failed, try again now!\r\n");
			sprintf(cmd, "fatload usb 0:0 ${loadaddr} update/%s", kernel);
			ret = run_command(cmd, 0);
		}
		if (ret) {
			printf("upgrade kernel failed, not found update/%s file!\r\n", kernel);
			goto err;
		}
	}

	if (strcmp(typename, "nand") == 0) {
		ret = mtdparts_probe();
		if (ret)
			goto err;
		printf("update kernel to nand............\n");
		ret = run_command("nand erase.part kernel;nand write ${loadaddr} kernel ${filesize}", 0);
		if (ret) {
			goto err;
		}
	} else if (strcmp(typename, "sata") == 0) {
		printf("update kernel to ssd.............\n");
		sprintf(cmd, "scsi reset;ext4write scsi 0:${syspart} ${loadaddr} /boot/%s ${filesize}", kernel);
		ret = run_command(cmd, 0);
		if (ret) {
			goto err;
		}
	} else {
		printf("upgrade kernel failed: nand or sata?\r\n");
		ret = -1;
		goto err;
	}

err:
	update_result_display(ret, dev, UPDATE_TYPE_KERNEL);

	return ret;
}

static int update_rootfs(int dev, char *typename)
{
	int ret = -1;

	printf("update rootfs to nand............\n");

	switch (dev) {
	case UPDATE_DEV_USB:
		ret = run_command("fatload usb 0:1 ${loadaddr} update/rootfs-ubifs-ze.img", 0);
		break;
	case UPDATE_DEV_TFTP:
		ret = run_command("tftpboot ${loadaddr} rootfs-ubifs-ze.img", 0);
		break;
	case UPDATE_DEV_MMC:
		ret = run_command("fatload mmc 0:1 ${loadaddr} update/rootfs-ubifs-ze.img", 0);
		break;
	case UPDATE_DEV_DHCP:
		ret = run_command("dhcp ${loadaddr} ${serverip}:rootfs-ubifs-ze.img", 0);
		break;
	}

	if (ret) {
		if (dev == UPDATE_DEV_USB) {
			printf("upgrade rootfs failed, try again now!\r\n");
			ret = run_command("fatload usb 0:0 ${loadaddr} update/rootfs-ubifs-ze.img", 0);
		}
		if (ret) {
			printf("upgrade rootfs failed, not found update/rootfs-ubifs-ze.img file!\r\n");
			return ret;
		}
	}

	ret = mtdparts_probe();
	if (ret) {
		return ret;
	}

	ret = run_command("nand erase.part root;nand write ${loadaddr} root ${filesize}", 0);

	update_result_display(ret, dev, UPDATE_TYPE_ROOTFS);

	return ret;
}

static int do_loongson_update(struct cmd_tbl *cmdtp, int flag, int argc, char * const argv[])
{
	int ret;
	int dev = UPDATE_DEV_UNKNOWN;
	int type = UPDATE_TYPE_UNKNOWN;

	if (!argv[1] || !argv[2]) {
		printf("\n");
		printf("######################################################\n");
		printf("### error! loongson_update need more param\n");
		goto err;
	}

	for (dev = 0; dev < UPDATE_DEV_COUNT; dev++) {
		if (update_devname_str[dev] &&
			!strcmp(argv[1], update_devname_str[dev]))
			break;
	}
	if ((dev == UPDATE_DEV_UNKNOWN) || (dev >= UPDATE_DEV_COUNT)) {
		printf("\n");
		printf("######################################################\n");
		printf("### upgrade dev unknown\r\n");
		printf("### please intput dev: usb\\tftp\\mmc\\dhcp\r\n");
		goto err;
	}

	for (type = 0; type < UPDATE_TYPE_COUNT; type++) {
		if (update_typename_str[type] &&
			!strcmp(argv[2], update_typename_str[type]))
			break;
	}
	if ((type == UPDATE_TYPE_UNKNOWN) || (type >= UPDATE_TYPE_COUNT)) {
		printf("\n");
		printf("######################################################\n");
		printf("### upgrade type unknown\r\n");
		printf("### please intput type: kernel\\rootfs\\uboot\\dtb\\all\r\n");
		goto err;
	}

	switch (dev) {
	case UPDATE_DEV_USB:
		run_command("usb reset", 0);
		ret = run_command("usb storage", 0);
		if (ret) {
			printf("### upgrade failed no usb storage!\r\n");
			goto err;
		}
		break;
	case UPDATE_DEV_TFTP:
		break;
	case UPDATE_DEV_MMC:
		ret = run_command("mmc rescan", 0);
		if (ret) {
			printf("### upgrade failed no mmc storage!\r\n");
			goto err;
		}
		break;
	case UPDATE_DEV_DHCP:
		break;
	default:
		break;
	}

	switch (type) {
	case UPDATE_TYPE_UBOOT:
		ret = update_uboot(dev);
		break;
	case UPDATE_TYPE_DTB:
		ret = update_dtb(dev);
		break;
	case UPDATE_TYPE_KERNEL:
		ret = update_kernel(dev, argv[3], argv[4]);
		break;
	case UPDATE_TYPE_ROOTFS:
		ret = update_rootfs(dev, NULL);
		break;
	case UPDATE_TYPE_ALL:
		// ret = update_dtb(dev);
		ret = update_kernel(dev, argv[3], argv[4]);
		ret = update_rootfs(dev, NULL);
		ret = update_uboot(dev);
		break;
	default:
		break;
	}

	return ret;

err:
	printf("######################################################\n");
	return -1;
}

U_BOOT_CMD(
	loongson_update,    5,    1,     do_loongson_update,
	"upgrade uboot kernel rootfs dtb over usb\\tftp\\mmc\\dhcp.",
	""
);
