#include <linux/types.h>
#include <log.h>
#include "ls_ddr.h"

#if defined(CONFIG_SPL_BUILD) || !defined(CONFIG_SPL)
u64 ls_ddr_param[] = {
/* MC0_DDR3_CTRL_0x000: */    0x0300000000000000,
/* MC0_DDR3_CTRL_0x008: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x010: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x018: */    0x4040404016100000,
/* MC0_DDR3_CTRL_0x020: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x028: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x030: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x038: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x040: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x048: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x050: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x058: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x060: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x068: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x070: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x078: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x080: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x088: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x090: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x098: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x0a0: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x0a8: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x0b0: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x0b8: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x0c0: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x0c8: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x0d0: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x0d8: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x0e0: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x0e8: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x0f0: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x0f8: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x100: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x108: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x110: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x118: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x120: */    0x0201000201000000,
/* MC0_DDR3_CTRL_0x128: */    0x0303000002010100,
/* MC0_DDR3_CTRL_0x130: */    0x0000000003020202,
/* MC0_DDR3_CTRL_0x138: */    0x0000002020056500,
/* MC0_DDR3_CTRL_0x140: */    0x0003000001ff01ff,
/* MC0_DDR3_CTRL_0x148: */    0x0000000000010100,
/* MC0_DDR3_CTRL_0x150: */    0x00020000f0020000,
/* MC0_DDR3_CTRL_0x158: */    0x00000000f0000000,
/* MC0_DDR3_CTRL_0x160: */    0x0000000000010001,
/* MC0_DDR3_CTRL_0x168: */    0x1400000707030101,
/* MC0_DDR3_CTRL_0x170: */    0x8421050084120501,
/* MC0_DDR3_CTRL_0x178: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x180: */    0x0000000001100000,
/* MC0_DDR3_CTRL_0x188: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x190: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x198: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x1a0: */    0x0000001000060d40,
/* MC0_DDR3_CTRL_0x1a8: */    0x0000001000060d40,
/* MC0_DDR3_CTRL_0x1b0: */    0x0000001000060d40,
/* MC0_DDR3_CTRL_0x1b8: */    0x0000001000060d40,
/* MC0_DDR3_CTRL_0x1c0: */    0x3030c80c03042004,
/* MC0_DDR3_CTRL_0x1c8: */    0x1107070715854080,
/* MC0_DDR3_CTRL_0x1d0: */    0x0a020c0402000019,//0x0a020c0302000019 //0x0a02090402000019,
/* MC0_DDR3_CTRL_0x1d8: */    0x14050c0608070406,
/* MC0_DDR3_CTRL_0x1e0: */    0x0503000000000000,
/* MC0_DDR3_CTRL_0x1e8: */    0x0309000000000000,
/* MC0_DDR3_CTRL_0x1f0: */    0x000801e4ff000101,
/* MC0_DDR3_CTRL_0x1f8: */    0x0000000004081001,
/* MC0_DDR3_CTRL_0x200: */    0x0c000c000c000c00,
/* MC0_DDR3_CTRL_0x208: */    0x0c000c0000000000,
/* MC0_DDR3_CTRL_0x210: */    0x0008010f00030006,
/* MC0_DDR3_CTRL_0x218: */    0x0008000b00030106,
/* MC0_DDR3_CTRL_0x220: */    0x0008000b00030106,
/* MC0_DDR3_CTRL_0x228: */    0x0008000b00030106,
/* MC0_DDR3_CTRL_0x230: */    0x0fff000000000000,
/* MC0_DDR3_CTRL_0x238: */    0x0ffffe000000ff00,
/* MC0_DDR3_CTRL_0x240: */    0x0ffffe000000ff00,
/* MC0_DDR3_CTRL_0x248: */    0x0ffffe000000ff00,
/* MC0_DDR3_CTRL_0x250: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x258: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x260: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x268: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x270: */    0x0000001000000000,
/* MC0_DDR3_CTRL_0x278: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x280: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x288: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x290: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x298: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2a0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2a8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2b0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2b8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2c0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2c8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2d0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2d8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2e0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2e8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2f0: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x2f8: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x300: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x308: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x310: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x318: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x320: */    0x0808301000016000,
/* MC0_DDR3_CTRL_0x328: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x330: */    0x0100011000000400,
/* MC0_DDR3_CTRL_0x338: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x340: */    0x0030d40000070f01,
/* MC0_DDR3_CTRL_0x348: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x350: */    0xffffffffffffffff,
/* MC0_DDR3_CTRL_0x358: */    0x000000000001ffff,
/* MC0_DDR3_CTRL_0x360: */    0x0000000000000000,
/* MC0_DDR3_CTRL_0x368: */    0x0000000000000000,
};

static void mc_update_param_val(u64 *param, int offset, int shift, u64 mask, u64 val)
{
	u64 v;
	int index = offset >> 3;
	int nshift = (offset & 0x7) * 8 + shift;

	if (nshift >= 64) {
		debug("ddr: update mc param offs: %d, shift: %d err(override)!\n", 
			offset, shift);
		return;
	}

	v = param[index];
	v &= ~(mask << nshift);
	v |= (val & mask) << nshift;
	param[index] = v;
}

void mc_adjust_param(struct mc_setting *mc)
{	
	u64 val;
	int i;

	if (mc->multi_channel) {
		//rewrite multi_channel mode
		mc_update_param_val(mc->params, MULTI_CHANNEL_ADDR, 
			MULTI_CHANNEL_OFFSET, MULTI_CHANNEL_MASK, MULTI_CHANNEL_EN);
	} else {
		//set data bus width
		mc_update_param_val(mc->params, DATA_WIDTH_ADDR, 
			DATA_WIDTH_OFFSET, DATA_WIDTH_MASK, mc->data_width);
	}

	for (i = 0; i < 4; i++) {
		//rewrite pm_addr_win(data width)
		val = 0;
		if (mc->sdram_banks == MC_SDRAM_BANK_8)
			val |= 0xc;
		else if (mc->sdram_banks == MC_SDRAM_BANK_4)
			val |= 0x8;
		else
			val |= 0x4;

		if (mc->data_width == MC_DATA_WIDTH_64)
			val |= 0x3;
		else if (mc->data_width == MC_DATA_WIDTH_32)
			val |= 0x2;
		else if (mc->data_width == MC_DATA_WIDTH_16)
			val |= 0x1;
		else
			val |= 0x3;

		mc_update_param_val(mc->params, (ADDR_WIN_OFFSET & (~0x7)) + i*8,
			(ADDR_WIN_OFFSET & 0x7) * 8, 0xF, val);

		//rewrite cs_diff_*
		val = 0x0;
		if (mc->data_width == MC_DATA_WIDTH_64)
			val = 0x2;
		else if (mc->data_width == MC_DATA_WIDTH_32)
			val = 0x1;
		mc_update_param_val(mc->params, (CS_DIFF_OFFSET & (~0x7)) + i*8,
			(CS_DIFF_OFFSET & 0x7) * 8, 0xF, val);
	}

	//disable ECC module here for leveling, ECC will be enabled later
	mc_update_param_val(mc->params, ECC_ENABLE_ADDR,
			ECC_ENABLE_OFFSET, 0x7, MC_ECC_EN_NO);

	// dll_clk_delay
	mc_update_param_val(mc->params, 0x18, 32, 0xff, mc->clk_latency);
	mc_update_param_val(mc->params, 0x18, 40, 0xff, mc->clk_latency);
	mc_update_param_val(mc->params, 0x18, 48, 0xff, mc->clk_latency);
	mc_update_param_val(mc->params, 0x18, 56, 0xff, mc->clk_latency);

	// rd latency
	mc_update_param_val(mc->params, 0x1d0, 40, 0xf, mc->rd_latency);
	// wr latency
	mc_update_param_val(mc->params, 0x1d0, 32, 0xf, mc->wr_latency);

	if (mc->reset_revert)
		mc_update_param_val(mc->params, 0x150,
			48, 0xff, 0);

	mc_update_param_val(mc->params, 0x160, 8, 0x3, mc->cmd_timing_mode);

	if ((mc->dimm_type == MC_DIMM_TYPE_UDIMM) && mc->addr_mirror)
		mc_update_param_val(mc->params, 0x168, 48, 0xff, 0xa);
	else
		mc_update_param_val(mc->params, 0x168, 48, 0xff, 0);
}
#endif