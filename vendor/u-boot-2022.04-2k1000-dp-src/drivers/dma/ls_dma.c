// SPDX-License-Identifier: GPL-2.0+
/*
 * Copyright (C) 2022 Loongson-GD <www.loongson.cn>
 *
 * Author: Jianhui Wang <wangjianhui@loongson.cn>
 */
#include <common.h>
#include <dm.h>
#include <malloc.h>
#include <dm/read.h>
#include <dm/of_addr.h>
#include <dm/ofnode.h>
#include <dma-uclass.h>
#include <asm/io.h>
#include <errno.h>
#include <linux/delay.h>
#include <dt-bindings/dma/loongson-dma.h>

#define LS_DMA_64BIT_EN		BIT(0)
#define LS_DMA_COHERENT		BIT(1)
#define LS_DMA_ASK_VALID	BIT(2)
#define LS_DMA_START		BIT(3)
#define LS_DMA_STOP			BIT(4)

#define LS_DMA_CMD_MEM_TO_DEV	(0x1 << 12)
#define LS_DMA_CMD_DEV_TO_MEM	(0x0 << 12)
#define LS_DMA_CMD_TRANS_OVER	BIT(3)
#define LS_DMA_CMD_INT			(0x1 << 1)
#define LS_DMA_CMD_INT_MASK		(0x1 << 0)

#define LS_DMA_MAX_CHANS	5
#define LS_DMA_DESC_ALIGN	32

enum {
	DMA_LS1X,
	DMA_LS2K1000,
	DMA_LS2K500,
};

struct ls_dmacfg_setting {
	int owner;
	unsigned int reg_shift;
	unsigned int reg_mask;
};

struct ls_dma_cfg {
	ofnode node;
	struct ls_dmacfg_setting *setting;
	void __iomem *base;
	int channel_id;
	int owner;
	void *rx_buf;
	size_t rx_sz;
};

struct ls_dma_desc {
	u32 dma_next_desc;
	u32 dma_saddr;
	u32 dma_daddr;
	u32 dma_len;
	u32 dma_step_len;
	u32 dma_step_times;
	u32 dma_cmd;
	u32 reserved;
	u32 dma_next_desc_h;
	u32 dma_saddr_h;
	u32 space[2];	// for align to 16 bytes
};

struct ls_dma_priv {
	void *base;
	int n_channels;
	ulong match_data;
	struct ls_dma_cfg cfg;
	struct ls_dma_desc *desc;
};

static struct ls_dmacfg_setting ls2k_reg_setting[] = {
	{LS_DMA_OWNER_NAND,		0, 0x7},
	{LS_DMA_OWNER_AES_R,	3, 0x7},
	{LS_DMA_OWNER_AES_W,	6, 0x7},
	{LS_DMA_OWNER_DES_R,	9, 0x7},
	{LS_DMA_OWNER_DES_W,	12, 0x7},
	{LS_DMA_OWNER_SDIO0,	15, 0x7},
	{LS_DMA_OWNER_I2S_TX,	18, 0x7},
	{LS_DMA_OWNER_I2S_RX,	21, 0x7}
};

static struct ls_dmacfg_setting ls2k500_reg_setting[] = {
	{LS_DMA_OWNER_NAND,		0, 0x0},
	{LS_DMA_OWNER_SDIO0,	0, 0x0},
	{LS_DMA_OWNER_SDIO1,	14, 0x3}
};

static struct ls_dma_desc __aligned(LS_DMA_DESC_ALIGN) ls_dma_desc[LS_DMA_MAX_CHANS];

static int ls_dma_config(struct ls_dma_priv *priv)
{
	struct ls_dma_cfg *dma_cfg = &priv->cfg;
	struct ls_dmacfg_setting *setting = NULL;
	u32 val;
	int i;

	if (!dma_cfg->base)
		return -EINVAL;

	switch (priv->match_data) {
	case DMA_LS1X:
		// dma channel owner can not changed!
		setting = NULL;
		break;
	case DMA_LS2K1000:
		for (i = 0; i < ARRAY_SIZE(ls2k_reg_setting); i++) {
			if (dma_cfg->owner == ls2k_reg_setting[i].owner)
				setting = &ls2k_reg_setting[i];
		}
		if (setting && setting->reg_mask > 0) {
			val = readl(dma_cfg->base);
			val &= ~(setting->reg_mask << setting->reg_shift);
			val |= ((dma_cfg->channel_id) & setting->reg_mask) << setting->reg_shift;
			writel(val, dma_cfg->base);
		}
		break;
	case DMA_LS2K500:
		for (i = 0; i < ARRAY_SIZE(ls2k500_reg_setting); i++) {
			if (dma_cfg->owner == ls2k500_reg_setting[i].owner)
				setting = &ls2k500_reg_setting[i];
		}
		if (setting && setting->reg_mask > 0) {
			val = readl(dma_cfg->base);
			val &= ~(setting->reg_mask << setting->reg_shift);
			val |= ((dma_cfg->channel_id + 1) & setting->reg_mask) << setting->reg_shift;
			writel(val, dma_cfg->base);
		}
		break;
	default:
		printf("dma config error!\n");
	}

	dma_cfg->setting = setting;
	return 0;
}

static int ls_dma_wait_complete(struct ls_dma_priv *priv, int chan_id)
{
	int timeout = 50;

	while (timeout > 0) {
		if (!(readl(priv->base) & LS_DMA_START))
			break;
		
		udelay(100);
		timeout--;
	}

	if (timeout <= 0) {
		debug("DMA timeout!\n");
		return -ETIMEDOUT;
	}

	return 0;
}

static int ls_dma_of_xlate(struct dma *dma, struct ofnode_phandle_args *args)
{
	// if (args->args_count)
	// 	dma->id = args->args[0];
	// else
	// 	dma->id = 0;

	if (args->args[0] >= 1)
		return -EINVAL;

	dma->id = args->args[0];

	return 0;
}

static int ls_dma_request(struct dma *dma)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);
	int res;

	res = ls_dma_config(priv);
	if (res)
		return res;

	// stop dma transform
	writel(LS_DMA_STOP, priv->base + dma->id);

	priv->desc = (struct ls_dma_desc *)ls_dma_desc;

	return 0;
}

static int ls_dma_rfree(struct dma *dma)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);
	// stop dma transform
	writel(LS_DMA_STOP, priv->base + dma->id);
	return 0;
}

static int ls_dma_enable(struct dma *dma)
{
	return 0;
}

static int ls_dma_disable(struct dma *dma)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);
	writel(LS_DMA_STOP, priv->base + dma->id);
	return 0;
}

static int ls_dma_send(struct dma *dma, void *src, size_t len, void *metadata)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);
	volatile struct ls_dma_desc *desc = priv->desc;
	phys_addr_t dev_addr = (phys_addr_t)metadata;
	u32 val;
	int res;

	if (!dev_addr)
		return -EINVAL;
	
	if ((phys_addr_t)desc % LS_DMA_DESC_ALIGN) {
		printf("dma addr not aligned\n");
		return -EINVAL;
	}

	desc->dma_next_desc = 0x0;
	desc->dma_saddr = (u32)virt_to_dma(src);
	desc->dma_daddr = (u32)(dev_addr & 0x0FFFFFFF);
	desc->dma_len = DIV_ROUND_UP(len, 4);
	desc->dma_step_len = 0;
	desc->dma_step_times = 1;
	desc->dma_cmd = LS_DMA_CMD_MEM_TO_DEV | LS_DMA_CMD_INT_MASK;

	val = (u32)virt_to_dma((void*)desc) & (~0x1F);
	val |= LS_DMA_START;
	writel(val, priv->base);

	res = ls_dma_wait_complete(priv, dma->id);
	if (res)
		return res;

	return 0;
}

static int ls_dma_receive(struct dma *dma, void **dst, void *metadata)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);
	volatile struct ls_dma_desc *desc = priv->desc;
	phys_addr_t dev_addr = (phys_addr_t)metadata;
	u32 val;
	int res;
	
	if (!dev_addr || !priv->cfg.rx_buf)
		return -EINVAL;
	
	if ((phys_addr_t)desc % LS_DMA_DESC_ALIGN) {
		printf("dma addr not aligned\n");
		return -EINVAL;
	}

	desc->dma_next_desc = 0x0;
	desc->dma_saddr = (u32)virt_to_dma(priv->cfg.rx_buf);
	desc->dma_daddr = (u32)(dev_addr & 0x0FFFFFFF);
	desc->dma_len = DIV_ROUND_UP(priv->cfg.rx_sz, 4);
	desc->dma_step_len = 0;
	desc->dma_step_times = 1;
	desc->dma_cmd = LS_DMA_CMD_DEV_TO_MEM | LS_DMA_CMD_INT_MASK;

	val = (u32)virt_to_dma(desc) & (~0x1F);
	val |= LS_DMA_START;
	writel(val, priv->base);

	res = ls_dma_wait_complete(priv, dma->id);
	if (res)
		return res;

	if (dst)
		*dst = priv->cfg.rx_buf;
	return 0;
}

static int ls_dma_prepare_rcv_buf(struct dma *dma, void *dst, size_t size)
{
	struct ls_dma_priv *priv = dev_get_priv(dma->dev);

	priv->cfg.rx_buf = dst;
	priv->cfg.rx_sz = size;
	return 0;
}


static int ls_dma_probe(struct udevice *dev)
{
	struct dma_dev_priv *uc_priv = dev_get_uclass_priv(dev);
	struct ls_dma_priv *priv = dev_get_priv(dev);
	struct ofnode_phandle_args dma_cfg;
	phys_addr_t cfg_base;
	int res;

	uc_priv->supported = (DMA_SUPPORTS_DEV_TO_MEM |
					DMA_SUPPORTS_MEM_TO_DEV);
	priv->match_data = dev_get_driver_data(dev);

	priv->base = dev_remap_addr(dev);
	if (!priv->base)
		return -ENODEV;

	priv->n_channels = dev_read_u32_default(dev, "dma-channels", 1);
	res = ofnode_parse_phandle_with_args(dev->node_, "dma-cfg", 
			"#config-cells", 2, 0, &dma_cfg);
	if (res) {
		printf("get dma config phandle failed: %d\n", res);
		return -EINVAL;
	}

	priv->cfg.node = dma_cfg.node;
	priv->cfg.channel_id = dma_cfg.args[0];
	priv->cfg.owner = dma_cfg.args[1];
	cfg_base = ofnode_get_addr(dma_cfg.node);
	if (cfg_base == FDT_ADDR_T_NONE) {
		printf("get dma config reg base failed\n");
		priv->cfg.base = NULL;
	} else {
		priv->cfg.base = (void __iomem *)cfg_base;
	}

	return 0;
}

static const struct dma_ops ls_dma_ops = {
	.of_xlate	= ls_dma_of_xlate,
	.request	= ls_dma_request,
	.rfree		= ls_dma_rfree,
	.enable		= ls_dma_enable,
	.disable	= ls_dma_disable,
	.send		= ls_dma_send,
	.receive	= ls_dma_receive,
	.prepare_rcv_buf = ls_dma_prepare_rcv_buf,
};

static const struct udevice_id ls_dma_ids[] = {
	{ .compatible = "loongson,ls1x-dma", .data = DMA_LS1X },
	{ .compatible = "loongson,ls2k1000-dma", .data = DMA_LS2K1000 },
	{ .compatible = "loongson,ls2k500-dma", .data = DMA_LS2K500 },
	{ }
};

U_BOOT_DRIVER(ls_dma) = {
	.name	= "ls-dma",
	.id	= UCLASS_DMA,
	.of_match = ls_dma_ids,
	.ops	= &ls_dma_ops,
	.probe = ls_dma_probe,
	.priv_auto	= sizeof(struct ls_dma_priv),
};