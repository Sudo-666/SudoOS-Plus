#include <config.h>
#include <asm/global_data.h>
#include <asm/io.h>
#include <mach/loongson.h>

DECLARE_GLOBAL_DATA_PTR;

void get_clocks(void)
{
    gd->cpu_clk = CORE_FREQ * 1000 * 1000;
	gd->mem_clk = DDR_FREQ * 1000 * 1000;
	gd->bus_clk = APB_FREQ * 1000 * 1000;
}