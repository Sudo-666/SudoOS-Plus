// SPDX-License-Identifier: GPL-2.0+

#include <common.h>
#include <command.h>
#include <stdlib.h>

#include <asm/gpio.h>

static int run_recover_cmd(char *cmd)
{
	int ret = -1;
	char cmdbuf[CONFIG_SYS_CBSIZE];	/* working copy of cmd		*/
	char *token;			/* start of token in cmdbuf	*/
	char *sep;			/* end of token (separator) in cmdbuf */
	char *str = cmdbuf;
	int inquotes;

	strcpy(cmdbuf, cmd);

	while (*str) {
		/*
		 * Find separator, or string end
		 * Allow simple escape of ';' by writing "\;"
		 */
		for (inquotes = 0, sep = str; *sep; sep++) {
			if ((*sep == '\'') &&
			    (*(sep - 1) != '\\'))
				inquotes = !inquotes;

			if (!inquotes &&
			    (*sep == ';') &&	/* separator		*/
			    (sep != str) &&	/* past string start	*/
			    (*(sep - 1) != '\\'))	/* and NOT escaped */
				break;
		}

		/*
		 * Limit the token to data between separators
		 */
		token = str;
		if (*sep) {
			str = sep + 1;	/* start of command for next pass */
			*sep = '\0';
		} else {
			str = sep;	/* no more commands for next pass */
		}

		ret = run_command(token, 0);
		if (ret) {
			printf("token: \"%s\" faile\n", token);
			break;
		}
	}

	return ret;
}

static int do_recover_from_usb(void)
{
	printf("Install System By USB .....\r\n");
	return run_recover_cmd(RECOVER_USB_DEFAULT);
}

static int do_recover_from_tftp(void)
{
	// char cmd[]= "tftpboot ${loadaddr} uImage;tftpboot ${rd_start} ramdisk.gz;"RECOVER_DEFAULT_ENV"";
	printf("Install System By tftp .....\r\n");
	return run_recover_cmd(RECOVER_TFTP_DEFAULT);
}

#ifdef CONFIG_MMC
static int do_recover_from_mmc(void)
{
	printf("Install System By MMC .....\r\n");
	return run_recover_cmd(RECOVER_MMC_DEFAULT);
}
#endif

static int do_recover_from_sata(char *part_id)
{
#ifdef RECOVER_SATA_DEFAULT
	printf("Recover System By SATA .....\r\n");
	return run_recover_cmd(RECOVER_SATA_DEFAULT);
#else
	char cmd[CONFIG_SYS_CBSIZE] = {0};
	snprintf(cmd, sizeof(cmd), "scsi reset;ext4load scsi 0:%s ${loadaddr} uImage;ext4load scsi 0:%s ${rd_start} ramdisk.gz;%s setenv bootargs ${bootargs} rec_sys=1; %s"
		, part_id, part_id, RECOVER_FRONT_BOOTARGS, RECOVER_START);
	return run_recover_cmd(cmd);
#endif
}

/* 上电时长按按钮3秒进入recover功能, recover优先顺序usb>mmc>sata
   按键使用 LS_RECOVER_GPIO_BUTTON 定义的gpio */
int recover(void)
{
#ifdef LS_RECOVER_GPIO_BUTTON
	int ret;
	int timeout = 0;

	gpio_request(LS_RECOVER_GPIO_BUTTON, "recover_butt");
	gpio_direction_input(LS_RECOVER_GPIO_BUTTON);

	while (gpio_get_value(LS_RECOVER_GPIO_BUTTON) == 0) {
		mdelay(10);
		timeout++;
		if (timeout > 300) {
			if (do_recover_from_usb() == 0) {
			} else if (do_recover_from_mmc() == 0) {
			} else {
				char part_id[4] = "4";
				do_recover_from_sata(part_id);
			}
			break;
		}
	}
#endif
	return 0;
}

#ifdef LS_DOUBLE_SYSTEM
static int do_recover_to_last(void)
{
	char *env;
	char *env_buf;
	int ret;

	env = env_get("bootcmd");
	if (strstr(env, "scsi") == NULL) {
		printf("current maybe is boot by nand\n");
		return -1;
	}

	//两个启动盘的情况 是 两个盘符之间的转换
	env = env_get("syspart_last");
	env_buf = malloc(strlen(env) + 1);
	sprintf(env_buf, "%s", env);
	env_set("syspart_last", env_get("syspart"));
	env_set("syspart", env_buf);
	env_save();

	ret = run_command(BOOT_SATA_DEFAULT, 0);
	if (ret)
		printf("recover last time system failed!\n");

	return ret;
}
#endif

/*
* recover_cmd usb . recover from usb
* recover_cmd sata. recover from sata
* recover_cmd sata 3. recover from part 3 of sata
*
* default table
* /dev/sda1 /
* /dev/sda2 /data
* /dev/sda3 swap
* /dev/sda4 backup
*
*/
static int do_recover_cmd(struct cmd_tbl *cmdtp, int flag, int argc, char * const argv[])
{
	int ret = -1;
	if (argc < 2) {
		return ret;
	}

	if (strcmp(argv[1], "usb") == 0) {
		ret = do_recover_from_usb();
	}
	else if (strcmp(argv[1], "tftp") == 0) {
		ret = do_recover_from_tftp();
	}
#ifdef CONFIG_MMC
	else if (strcmp(argv[1], "mmc") == 0) {
		ret = do_recover_from_mmc();
	}
#endif
	else if (strcmp(argv[1], "sata") == 0) {
		char part_id[4] = "4";
		if (argc == 3) {
			memset(part_id, 0, sizeof(part_id));
			int len = strlen(argv[2]) > sizeof(part_id) ? sizeof(part_id) : strlen(argv[2]);
			strncpy(part_id, argv[2], len);
		}
		printf("part_id:%s\n", part_id);
		ret = do_recover_from_sata(part_id);
	}
#ifdef LS_DOUBLE_SYSTEM
	else if (strcmp(argv[1], "last") == 0) {
		ret = do_recover_to_last();
	}
#endif

	return ret;
}

#define RECOVER_CMD_TIP_HEAD "recover system by usb or backup partition.\n"\
								"recover_cmd <option> [part_id]\n"\
								"option: usb: recover from usb\n"\
								"option: tftp: recover from tftp\n"

#ifdef CONFIG_MMC
#define RECOVER_CMD_TIP_MMC "        mmc: recover from mmc\n"
#else
#define RECOVER_CMD_TIP_MMC ""
#endif

#ifdef LS_DOUBLE_SYSTEM
#define RECOVER_CMD_TIP_LAST "        last: recover to last time boot system\n"
#else
#define RECOVER_CMD_TIP_LAST ""
#endif

#define RECOVER_CMD_TIP RECOVER_CMD_TIP_HEAD \
							RECOVER_CMD_TIP_MMC \
							RECOVER_CMD_TIP_LAST \
							"        sata: recover from sata\n"\
							"        part_id: recover from part id of sata\n"

U_BOOT_CMD(
	recover_cmd,    3,    1,     do_recover_cmd,
	"recover the system",
	RECOVER_CMD_TIP
);
