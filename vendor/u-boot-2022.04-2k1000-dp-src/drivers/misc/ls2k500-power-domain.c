// SPDX-License-Identifier: GPL-2.0
/*
 * Copyright 2022 loongson(GD)
 */
#include <common.h>
#include <dm.h>
#include <misc.h>
#include <asm/io.h>

#define LS_DPM_EN_OFFSET	0x0
#define LS_DPM_TGT_OFFSET	0x8
#define LS_DPM_STS_OFFSET	0xc


struct ls2k500_domain_plat_data {
	void __iomem *base;
};

static void dpm_enable_set(struct ls2k500_domain_plat_data *priv, u32 sel)
{
	u32 val = 0;
	val = readl(priv->base + LS_DPM_EN_OFFSET);
	val |= (0x1 << sel);
	writel(val, priv->base);
}

static void dpm_enable_get(struct ls2k500_domain_plat_data *priv)
{
	u32 val = 0;
	val = readl(priv->base + LS_DPM_EN_OFFSET);
	printf("dpm_en:0x%x\n", val);
}

/*
 * @mask: bit mask of bits to clear
 * @set: bit mask of bits to set
*/
static void dpm_tgt_set(struct ls2k500_domain_plat_data *priv, u32 mask, u32 set)
{
	u32 val = 0;
	val = readl(priv->base + LS_DPM_TGT_OFFSET);
	val = (val & ~mask) | set;
	writel(val, priv->base + LS_DPM_TGT_OFFSET);
}

static void dpm_sts_get(struct ls2k500_domain_plat_data *priv)
{
	u32 val = 0;
	val = readl(priv->base + LS_DPM_STS_OFFSET);
	printf("dpm_sts:0x%x\n", val);
}

static int ls2k500_power_domain_read(struct udevice *dev, int offset, void *buf, int size)
{
	printf("lsdbg-->%s\n", __func__);

	return 0;
}

static int ls2k500_power_domain_write(struct udevice *dev, int offset, const void *buf,
		       int size)
{
	printf("lsdbg-->%s\n", __func__);

	return 0;
}

static int ls2k500_power_domain_ioctl(struct udevice *dev, unsigned long request, void *buf)
{
	printf("lsdbg-->%s\n", __func__);
	return 0;
}

static int ls2k500_power_domain_call(struct udevice *dev, int msgid, void *tx_msg,
		      int tx_size, void *rx_msg, int rx_size)
{
	printf("lsdbg-->%s \n", __func__);
	return 0;
}


static int ls2k500_power_domain_set_enabled(struct udevice *dev, bool val)
{
	printf("lsdbg-->%s \n", __func__);
	return 0;
}

static int ls2k500_power_domain_parse(struct udevice *parent, struct ls2k500_domain_plat_data *data)
{
	ofnode node;
	u32 offset;
	u32 mask;
	u32 value;
	u32 dpm_en;
	int ret;

	dev_for_each_subnode(node, parent) {
		ret = ofnode_read_u32(node, "offset", &offset);
		if (ret) {
			printf("%s offset node read err.\n", __func__);
		}
		ret = ofnode_read_u32(node, "mask", &mask);
		if (ret) {
			printf("%s mask node read err.\n", __func__);
		}
		ret = ofnode_read_u32(node, "value", &value);
		if (ret) {
			printf("%s vaule node read err.\n", __func__);
		}
		ret = ofnode_read_u32(node, "dpm_en", &dpm_en);
		if (ret) {
			printf("%s dpm_en node read err.\n", __func__);
		}

		debug("offset:%d, mask:0x%x, value:%d, dpm_en:%d\n" 
		, offset, mask, value, dpm_en);

		if (dpm_en) {
			dpm_enable_set(data, offset);
			u32 set = value << (offset*2);
			dpm_tgt_set(data, mask, set);
		}
	};

	return 0;
}

static int ls2k500_power_domain_probe(struct udevice *dev)
{
	struct ls2k500_domain_plat_data *priv = dev_get_priv(dev);
	priv->base = dev_read_addr_ptr(dev);
	if (!priv->base)
		return -ENOENT;

	ls2k500_power_domain_parse(dev, priv);
	dpm_enable_get(priv);
	dpm_sts_get(priv);

	return 0;
}


static const struct udevice_id ls2k500_power_domain_ids[] = {
	{ .compatible = "loongson,dpm" },
	{ }
};

static const struct misc_ops ls2k500_power_domain_ops = {
	.read = ls2k500_power_domain_read,
	.write = ls2k500_power_domain_write,
	.ioctl = ls2k500_power_domain_ioctl,
	.call = ls2k500_power_domain_call,
	.set_enabled = ls2k500_power_domain_set_enabled,
};

U_BOOT_DRIVER(misc_ls2k500_power) = {
	.name = "misc_ls2k500_power",
	.id = UCLASS_MISC,
	.of_match = ls2k500_power_domain_ids,
	.probe = ls2k500_power_domain_probe,
	.priv_auto	= sizeof(struct ls2k500_domain_plat_data),
	.ops = &ls2k500_power_domain_ops,
};
