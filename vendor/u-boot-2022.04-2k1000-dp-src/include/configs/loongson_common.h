/* SPDX-License-Identifier: GPL-2.0+ */

#ifndef __LOONGSON_ENV_H__
#define __LOONGSON_ENV_H__


#if defined(CONFIG_SOC_LS2K500)
#define CMDLINE_CONSOLE		"console=ttyS2,115200"
#else
#define CMDLINE_CONSOLE		"console=ttyS0,115200"
#endif

#ifdef CONFIG_64BIT
#define FDT_ADDR    0x900000000a000000
#define RD_ADDR     0x9000000007000000
#else
#define FDT_ADDR    0x0a000000
#define RD_ADDR     0x07000000
#endif

#define RD_SIZE		0x2000000 /* ramdisk size:32M == 32768K*/

#define LOONGSON_BOOTMENU \
	"menucmd=bootmenu\0" \
	"bootmenu_0=System boot select=updatemenu bootselect 1\0" \
	"bootmenu_1=Update kernel=updatemenu kernel 1\0" \
	"bootmenu_2=Update rootfs=updatemenu rootfs 1\0" \
	"bootmenu_3=Update u-boot=updatemenu uboot 1\0" \
	"bootmenu_4=Update ALL=updatemenu all 1\0" \
	"bootmenu_5=System install or recover=updatemenu system 1\0" \
	"bootmenu_delay=10\0"

#define BOOTMENU_END "Return u-boot console"

#ifdef CONFIG_VIDEO
#define CONSOLE_STDOUT_SETTINGS \
	"stdin=serial,usbkbd\0" \
	"stdout=serial\0" \
	"stderr=serial,vga\0"
#elif defined(CONFIG_DM_VIDEO)
#define CONSOLE_STDOUT_SETTINGS \
	"splashimage=" __stringify(CONFIG_SYS_LOAD_ADDR) "\0" \
	"stdin=serial,usbkbd\0" \
	"stdout=serial\0" \
	"stderr=serial,vidconsole,vidconsole1\0"
#else
#define CONSOLE_STDOUT_SETTINGS \
	"stdin=serial\0" \
	"stdout=serial\0" \
	"stderr=serial\0"
#endif

#define RECOVER_FRONT_BOOTARGS "setenv bootargs " CMDLINE_CONSOLE " rd_start=${rd_start} rd_size=${rd_size} \
mtdparts=${mtdparts} root=/dev/ram init=/linuxrc rw rootfstype=ext2 video=${video};"

#define RECOVER_START "bootm ${loadaddr}"

#define RECOVER_USB_DEFAULT "usb reset;fatload usb 0:1 ${loadaddr} /install/uImage;fatload usb 0:1 ${rd_start} /install/ramdisk.gz;"\
RECOVER_FRONT_BOOTARGS "setenv bootargs ${bootargs} ins_way=usb;" RECOVER_START

#define RECOVER_MMC_DEFAULT "mmc rescan;fatload mmc 0:1 ${loadaddr} /install/uImage;fatload mmc 0:1 ${rd_start} /install/ramdisk.gz;"\
RECOVER_FRONT_BOOTARGS "setenv bootargs ${bootargs} ins_way=mmc;" RECOVER_START

#define RECOVER_TFTP_DEFAULT "tftpboot ${loadaddr} uImage;tftpboot ${rd_start} ramdisk.gz;"\
RECOVER_FRONT_BOOTARGS "setenv bootargs ${bootargs} ins_way=tftp u_ip=${ipaddr} u_sip=${serverip};" RECOVER_START

#define SATA_BOOT_ENV "setenv bootargs " CMDLINE_CONSOLE " rootfstype=ext4 rw rootwait; \
setenv bootcmd ' setenv bootargs ${bootargs} root=/dev/sda${syspart} mtdparts=${mtdparts} video=${video}; \
sf probe;sf read ${fdt_addr} dtb;scsi reset;ext4load scsi 0:${syspart} ${loadaddr} /boot/uImage;bootm ';\
saveenv;"

#define BOOT_SATA_DEFAULT SATA_BOOT_ENV"boot"

#define NAND_BOOT_ENV "setenv bootargs " CMDLINE_CONSOLE " root=ubi0:rootfs noinitrd init=/linuxrc rootfstype=ubifs rw; \
setenv bootcmd ' setenv bootargs ${bootargs} ubi.mtd=root,${nand_pagesize} mtdparts=${mtdparts} video=${video}; \
sf probe;sf read ${fdt_addr} dtb;nboot kernel;bootm ';\
saveenv;"

#define BOOT_NAND_DEFAULT NAND_BOOT_ENV"boot"

#define BOOT_SATA_CFG_DEFAULT "setenv bootcmd ' bootcfg scsi ';\
saveenv;\
boot"

#define BOOT_USB_CFG_DEFAULT "setenv bootcmd ' bootcfg usb ';\
saveenv;\
boot"

#define	CONFIG_EXTRA_ENV_SETTINGS					\
	CONSOLE_STDOUT_SETTINGS \
	LOONGSON_BOOTMENU \
	"nand_pagesize=2048\0" \
	"loadaddr=" __stringify(CONFIG_SYS_LOAD_ADDR) "\0" 		\
	"fdt_addr=" __stringify(FDT_ADDR) "\0" 					\
	"rd_start=" __stringify(RD_ADDR) "\0" 					\
	"rd_size=" __stringify(RD_SIZE) "\0" 					\
	"mtdids=" CONFIG_MTDIDS_DEFAULT "\0"					\
	"mtdparts=" CONFIG_MTDPARTS_DEFAULT "\0"				\
	"splashpos=m,m\0" \
	"video=" "VGA-1:1280x800-32@60 video=VGA-2:1280x800-32@60" "\0" \
	"syspart=1\0" \
	"update=1\0" \

#define CONFIG_IPADDR		192.168.1.20
#define CONFIG_NETMASK		255.255.255.0
#define CONFIG_SERVERIP		192.168.1.2

#endif /* __LOONGSON_ENV_H__ */
