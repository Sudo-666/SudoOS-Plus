/* SPDX-License-Identifier: GPL-2.0+ */
/*
 * Copyright (C) 2022 Loongson(GD) <maoxiaochuan@loongson.cn>
 */

#ifndef __DT_BINDINGS_POWER_DOMAIN_LS2K500_H
#define __DT_BINDINGS_POWER_DOMAIN_LS2K500_H

#define LS2K500_DPM_NODE	0
#define LS2K500_DPM_DDR		1
#define LS2K500_DPM_PCI		2
#define LS2K500_DPM_PCIE	3
#define LS2K500_DPM_GPU		4
#define LS2K500_DPM_DC		5
#define LS2K500_DPM_GMAC0	6
#define LS2K500_DPM_GMAC1	7
#define LS2K500_DPM_SATA	8
#define LS2K500_DPM_USB		9
#define LS2K500_DPM_USB3	10
#define LS2K500_DPM_HDA		11
#define LS2K500_DPM_PRINT	12

#define LS2K500_DPM_TGT_ON   	    0x0
#define LS2K500_DPM_TGT_CLKOFF 	    0x1
#define LS2K500_DPM_TGT_POWEROFF    0x3

#endif /* __DT_BINDINGS_POWER_DOMAIN_LS2K500_H */
