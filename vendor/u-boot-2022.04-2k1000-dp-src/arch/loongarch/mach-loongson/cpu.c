/*
 * Copyright (C) 2022
 * <wangjianhui@loongson.cn>
 *
 * SPDX-License-Identifier:	GPL-2.0+
 *
 */
#include <common.h>
#include <dm.h>
#include <clk.h>
#include <clk-uclass.h>
#include <linux/delay.h>
#include <asm/io.h>
#include <asm/addrspace.h>

DECLARE_GLOBAL_DATA_PTR;

extern void get_clocks(void);


int mach_cpu_init(void)
{
	get_clocks();
	return 0;
}


const char *get_core_name(void)
{
	u32 prid;
	const char *str;

	prid = gd->arch.cpuprid;
	switch (prid) {
	case 0x0014a000:
		str = "LA264";
		break;
	default:
		str = "Unknown";
	}

	return str;
}

int print_cpuinfo(void)
{
	printf("CPU:   %s\n", get_core_name());
	printf("Speed: Cpu @ %ld MHz/ Mem @ %ld MHz/ Bus @ %ld MHz\n",
			gd->cpu_clk/1000000, gd->mem_clk/1000000, gd->bus_clk/1000000);
	return 0;
}
