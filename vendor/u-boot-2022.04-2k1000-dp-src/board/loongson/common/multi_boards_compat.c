#include <image.h>
#include <common.h>
#include <asm/global_data.h>
#include <dm/root.h>
#include <i2c_eeprom.h>
#include <dm/uclass.h>
#include <dm/device.h>
#include <dm/lists.h>
#include <asm/io.h>

DECLARE_GLOBAL_DATA_PTR;

#define LS_HW_CODE_SZ	32

struct ls_board_info {
	char *board_name;
	char *dtb_name;
	char *hw_code;
};

static struct ls_board_info bi_default = {
	.board_name = "LS2K1000-Default",
	.dtb_name = "ls2k1000_defualt",
	.hw_code = "default",
};

static struct ls_board_info board_info_table[] = {
	{"LS2K1000-JinLong-MB-V1.0", "ls2k1000_jinlong_mb_v1.1", "JL-LSGD2K10-MBM1-V10"},
	{"LS2K1000-JinLong-MB-V1.1", "ls2k1000_jinlong_mb_v1.1", "JL-LSGD2K10-MBM1-V11"},
	{"LS2K1000-JinLong-MB-V1.2", "ls2k1000_jinlong_mb_v1.2", "JL-LSGD2K10-MBM1-V12"},
	{"LS2K1000-JinLong-MB-V1.3", "ls2k1000_jinlong_mb_v1.3", "JL-LSGD2K10-MBM1-V13"},
	{"LS2K1000-JL-LSGD2K10-BPC1", "ls2k1000_jl_lsgd2k10_bpc1", "JL-LSGD2K10-BPC1-V10"},
	{NULL, NULL, NULL}
};

static struct ls_board_info  *cur_board_info = NULL;

static int get_board_info(void)
{
	struct ls_board_info *bi;
	char buf[LS_HW_CODE_SZ + 1];
	struct udevice *dev;
	int res;
	u32 val;

	memset(buf, 0, sizeof(buf));

	// i2c mux function config.
	// during early boot, pinctrl is not ready.
	// val = readl((unsigned int *)CKSEG1ADDR(LS2X_MUX_BASE));
	// val |= (0x1 << 10) | (1<<11);
	// writel(val, (unsigned int *)CKSEG1ADDR(LS2X_MUX_BASE));

	res = uclass_first_device(UCLASS_I2C_EEPROM, &dev);
	if (res) {
		printf("get i2c-eeprom device failed: %d\n", res);
		goto failed;
	}

	do {
		if ( i2c_eeprom_read(dev, 0, (uint8_t*)buf, LS_HW_CODE_SZ) ) {
			printf("read eeprom %s failed: %d\n", dev->name, res);
			continue;
		}

#ifdef DEBUG
		printf("eeprom: ");
		for (int i = 0; i < LS_HW_CODE_SZ; i++) {
			printf(" 0x%X", buf[i]);
		}
		printf("\n");
#endif

		for (bi = board_info_table; bi->hw_code; bi++) {
			if (strncmp(buf, bi->hw_code, strlen(bi->hw_code)) == 0) {
				cur_board_info = bi;
				strlcpy((char*)gd->arch.board_name, bi->board_name,
							sizeof(gd->arch.board_name));
				debug("cur board info: %s, dtb: %s, hw: %s\n", cur_board_info->board_name,
							cur_board_info->dtb_name, cur_board_info->hw_code);
				return 0;
			}
		}
	} while (!uclass_next_device_err(&dev));

failed:
	cur_board_info = &bi_default;
	strlcpy((char*)gd->arch.board_name, bi_default.board_name,
				sizeof(gd->arch.board_name));
	return -ENOENT;
}

#ifdef CONFIG_MULTI_DTB_FIT
int board_fit_config_name_match(const char *name)
{
	struct ls_board_info *bi;
	if (!cur_board_info && !strncmp(name, "ls2k1000_default", strlen(name)))
		return 0;

	bi = cur_board_info;

	if (!bi || bi->dtb_name == NULL) {
		printf("dtb error!\n");
		return -1;
	}

	if (strcmp(name, bi->dtb_name) == 0) {
		printf("Found the real dtb: %s.dtb\n", name);
		return 0;
	}

	debug("Mismatch dtb: %s.dtb!!\n", name);
	return -1;
}
#endif

#if defined(CONFIG_DTB_RESELECT)
int embedded_dtb_select(void)
{
	int res;
	int rescan = 0;

	get_board_info();

	res = fdtdec_resetup(&rescan);
	if (!res && rescan) {
		dm_uninit();
		dm_init_and_scan(true);
	}

	return 0;
}
#endif