// SPDX-License-Identifier: GPL-2.0+
/*
 * Copyright (C) 2022, Jianhui Wang <wangjianhui@loongson.cn>
 */

#include <common.h>
#include <ahci.h>
#include <scsi.h>
#include <dm.h>
#include <asm/io.h>

static int ls_ahci_bind(struct udevice *dev)
{
	struct udevice *scsi_dev;

	return ahci_bind_scsi(dev, &scsi_dev);
}

static int ls_ahci_probe(struct udevice *dev)
{
	void __iomem *base;

	base = ioremap((resource_size_t)dev_read_addr(dev), sizeof(void *));

	return ahci_probe_scsi(dev, (ulong)base);
}

static const struct udevice_id ls_ahci_ids[] = {
	{ .compatible = "loongson,ls-ahci" },
	{ }
};

U_BOOT_DRIVER(ls_ahci) = {
	.name	= "ls_ahci",
	.id	= UCLASS_AHCI,
	.ops	= &scsi_ops,
	.of_match = ls_ahci_ids,
	.bind	= ls_ahci_bind,
	.probe = ls_ahci_probe,
};
