// SPDX-License-Identifier: GPL-2.0+
/*
 * Copyright (C) 2020 Tang Haifeng <tanghaifeng-gz@loongson.cn>
 */

#include <common.h>
#include <dm.h>
#include <errno.h>
#include <i2c.h>
#include <video_bridge.h>
#include <power/regulator.h>
#include <dm/device-internal.h>
#include <dm/uclass-internal.h>
#include <linux/delay.h>
#include <edid.h>

#define PIXEL_CLK_LSB_REG		0x00
#define PIXEL_CLK_MSB_REG		0x01
#define VERT_FREQ_LSB_REG		0x02
#define VERT_FREQ_MSB_REG		0x03
#define TOTAL_PIXELS_LSB_REG		0x04
#define TOTAL_PIXELS_MSB_REG		0x05
#define TOTAL_LINES_LSB_REG		0x06
#define TOTAL_LINES_MSB_REG		0x07
#define TPI_INBUS_FMT_REG		0x08
#define TPI_INPUT_FMT_REG		0x09
#define TPI_OUTPUT_FMT_REG		0x0A
#define TPI_SYS_CTRL_REG		0x1A
#define TPI_PWR_STAT_REG		0x1E
#define TPI_AUDIO_HANDING_REG		0x25
#define TPI_AUDIO_INTF_REG		0x26
#define TPI_AUDIO_FREQ_REG		0x27
#define TPI_SET_PAGE_REG		0xBC
#define TPI_SET_OFFSET_REG		0xBD
#define TPI_RW_ACCESS_REG		0xBE
#define TPI_TRANS_MODE_REG		0xC7

#if defined(CONFIG_TARGET_LS2K1000_CORE) || defined(CONFIG_TARGET_LS2K1000_JINLONG_MB_V1_1) || defined(CONFIG_TARGET_LS2K1000_JINLONG_MB_V1_0)
#if defined(CONFIG_DM_PCA953X)
#define RESET_GPIO 68
#else
#define RESET_GPIO 0
#endif
#elif defined(CONFIG_TARGET_LS2K1000_PAI)
#define RESET_GPIO 39
#elif defined(CONFIG_TARGET_LS2K1000_ICTRL)
#define RESET_GPIO 0
#endif

struct sii9022a {
	struct udevice *ddc_bus;
};

static int sii9022a_attach(struct udevice *dev)
{
	int ret, __maybe_unused i;
	unsigned char data = 0;
	unsigned char id0;
	unsigned char id1;
	unsigned char id2;

#if CONFIG_IS_ENABLED(DM_GPIO)
	struct gpio_desc reset_gpios;
	ret = gpio_request_by_name(dev, "reset_gpios", 0, &reset_gpios, GPIOD_IS_OUT);
	if (ret)
		return ret;
	dm_gpio_set_value(&reset_gpios, 0);
	udelay(120);
	dm_gpio_set_value(&reset_gpios, 1);
	udelay(120);
#else
	ret = gpio_request(RESET_GPIO, "sii9022a_reset");
	if (ret)
		return ret;
	gpio_direction_output(RESET_GPIO, 0);
	udelay(120);
	gpio_direction_output(RESET_GPIO, 1);
	udelay(120);
#endif

	ret = video_bridge_set_active(dev, true);
	if (ret)
		return ret;

	dm_i2c_write(dev, TPI_TRANS_MODE_REG, &data, 1);

	dm_i2c_read(dev, 0x1b, &id0, 1);
	dm_i2c_read(dev, 0x1c, &id1, 1);
	dm_i2c_read(dev, 0x1d, &id2, 1);
	debug("Sii9022a: ID 0x%02x 0x%02x 0x%02x\n", id0, id1,id2);

	if (id0 != 0xb0 || id1 != 0x2 || id2 != 0x3) {
		return -ENODEV;
	}

	dm_i2c_read(dev, TPI_PWR_STAT_REG, &data, 1);
	data &= ~(0x3);
	dm_i2c_write(dev, TPI_PWR_STAT_REG, &data, 1);

	/* Active TMDS Output */
	dm_i2c_read(dev, TPI_SYS_CTRL_REG, &data, 1);
	data &= ~(1 << 4);
	dm_i2c_write(dev, TPI_SYS_CTRL_REG, &data, 1);

	return 0;
}

static int sii9022a_read_edid(struct udevice *dev, u8 *buf, int size)
{
	struct sii9022a *priv = dev_get_priv(dev);
	struct udevice *chip;
	int ret;

	ret = i2c_get_chip(priv->ddc_bus, 0x50, 1, &chip);
	if (ret) {
		return -EINVAL;
	}

	ret = dm_i2c_read(chip, 0, buf, size);
	if (ret) {
		debug("video edid get failed: %d\n", ret);
		return -EINVAL;
	}

	return size;
}

static int sii9022a_probe(struct udevice *dev)
{
	struct sii9022a *priv = dev_get_priv(dev);
	int ret;
	
	ret = uclass_get_device_by_phandle(UCLASS_I2C, dev, "ddc-i2c-bus",
				     &priv->ddc_bus);
	if (ret) {
		printf("Cannot get ddc bus\n");
		priv->ddc_bus = NULL;
		return -EINVAL;
	}

	return 0;
}

struct video_bridge_ops sii9022a_ops = {
	.attach = sii9022a_attach,
	.read_edid = sii9022a_read_edid,
};

static const struct udevice_id sii9022a_ids[] = {
	{ .compatible = "latticesemi,sii9022a", },
	{ }
};

U_BOOT_DRIVER(sii9022a) = {
	.name	= "sii9022a",
	.id	= UCLASS_VIDEO_BRIDGE,
	.of_match = sii9022a_ids,
	.probe	= sii9022a_probe,
	.ops	= &sii9022a_ops,
	.priv_auto = sizeof(struct sii9022a),
};
