#ifndef __LOONGSON_DDR_H__
#define __LOONGSON_DDR_H__

#include <asm-generic/types.h>
#include <config.h>
#include <ram.h>

#define DQ_CHANGE_WITH_DQS

// ddr register
#define MC_REGS_COUNT		110
#define MC0_INDEX		0x0
#define MC1_INDEX		0x1
#define SLOT0_INDEX		0x0
#define SLOT1_INDEX		0x1
#define MC0_SLOT0_I2C_ADDR	0x0
#define MC0_SLOT1_I2C_ADDR	0xf
#define MC1_SLOT0_I2C_ADDR	0x1
#define MC1_SLOT1_I2C_ADDR	0xf

#define DLL_VALVE_CK_OFFSET	0x32
#define INIT_START_OFFSET	0x18

#define WRDQ_LT_HALF_OFFSET	0x20
#define WRDQS_LT_HALF_OFFSET	0x21
#define RDDQS_LT_HALF_OFFSET	0x22
#define RDDATA_DELAY_OFFSET	0x23
#define RDOE_BEGIN_OFFSET	0x2e
#define RDOE_END_OFFSET		0x2f
#define RDODT_BEGIN_OFFSET	0x32
#define RDODT_END_OFFSET	0x33
#define WRDQ_CLKDELAY_OFFSET	0x34
#define DLL_GATE_OFFSET		0x38
#define DLL_WRDQ_OFFSET		0x39
#define DLL_WRDQS_OFFSET	0x3a

#define DDR3_MODE_OFFSET	0x160
#define DRAM_INIT_OFFSET	0x163
#define CS_ENABLE_OFFSET	0x168
#define CS_MRS_OFFSET		0x169
#define CS_ZQ_OFFSET		0x16a
#define BURST_LENGTH_OFFSET	0x16c
#define ADDR_MIRROR_OFFSET	0x16e

#define COL_DIFF_OFFSET		0x210
#define BA_DIFF_OFFSET		0x211
#define ROW_DIFF_OFFSET		0x212
#define CS_DIFF_OFFSET		0x213
#define ADDR_WIN_OFFSET		0x214

#define ODT_MAP_OFFSET		0x170
#define ECC_ENABLE_ADDR		0x250
#define ECC_ENABLE_OFFSET	16

#define ADDR_INFO_CS_0_OFFSET	0x210
#define ADDR_INFO_CS_1_OFFSET	0x218
#define ADDR_INFO_CS_2_OFFSET	0x220
#define ADDR_INFO_CS_3_OFFSET	0x228

#define CMD_TIMING_OFFSET	0x161
#define tRDDATA_OFFSET		0x1c0
#define VERSION_OFFSET		0x0
#define BANK_OFFSET		0x16b

#define LVL_CS_OFFSET		0x183
#define LVL_MODE_OFFSET		0x180
#define LVL_REQ_OFFSET		0x181
#define LVL_READY_OFFSET	0x185
#define LVL_DONE_OFFSET		0x186
#define LVL_RESP_OFFSET		0x187

#define tPHY_RDDATA_OFFSET	0x1c0
#define tPHY_WRLAT_OFFSET	0x1d4

#define WLVL_CHECK_BIT		0x01
#define GLVL_CHECK_BIT		0x3
#define WLVL_FILTER_LEN		0x5
#define GLVL_FILTER_LEN		0x5
#define DLL_WRDQ_SUB		0x20
#define DLL_GATE_SUB		0x20
#define DLL_ADJ_RANGE		0x8
#define WRDQ_LT_HALF_STD	0x40
#define WRDQS_LT_HALF_STD	0x40
#define RDDQS_LT_HALF_STD1	0x40
#define RDDQS_LT_HALF_STD2	0x00
#define PREAMBLE_LEN		0x68

#define DATA_WIDTH_ADDR		(0x1f0)
#define DATA_WIDTH_OFFSET	16
#define DATA_WIDTH_MASK 	(0x7)
#define DATA_WIDTH_64		(0x0)
#define DATA_WIDTH_16		(0x3)
#define DATA_WIDTH_32		(0x5)

#define MULTI_CHANNEL_ADDR	(0x1f0)
#define MULTI_CHANNEL_OFFSET	16
#define MULTI_CHANNEL_MASK	(0x7)
#define MULTI_CHANNEL_EN	(0x1)
#define MULTI_CHANNEL_DIS	(0x0)

// #define ADDR_WIN_DATA_WIDTH_ADDR	(0x210)
// #define ADDR_WIN_DATA_WIDTH_OFFSET	32

#define CS_MAP_OFFSET		0x1f4

#define BURST_LENGTH		8

// #define DRAM_TYPE_OFFSET	0x2
// #define DDR2			0x8
// #define DDR3			0xb

// #define MODULE_TYPE_OFFSET	0x3
// #define RDIMM		0x1
// #define UDIMM		0x2
// #define SODIMM		0x3

#define DRAM_DEN_BANK_OFFSET	0x4
#define DRAM_DEN_OFFSET		0x0
#define DRAM_DEN_BIT		0xf
#define DRAM_DEN_256Mb		0x0
#define DRAM_DEN_512Mb		0x1
#define DRAM_DEN_1Gb		0x2
#define DRAM_DEN_2Gb		0x3
#define DRAM_DEN_4Gb		0x4
#define DRAM_DEN_8Gb		0x5
#define DRAM_DEN_16Gb		0x6
#define DRAM_BANK_OFFSET	0x4
#define DRAM_BANK_BIT		0x7
#define BANK_8			0x0
#define BANK_16			0x1

#define DRAM_ADDR_OFFSET	0x5
#define DRAM_COL_OFFSET		0x0
#define DRAM_COL_BIT		0x7
#define DRAM_COL_9		0x0
#define DRAM_COL_10		0x1
#define DRAM_COL_11		0x2
#define DRAM_COL_12		0x3
#define DRAM_ROW_OFFSET		0x3
#define DRAM_ROW_BIT		0x7
#define DRAM_ROW_12		0x0
#define DRAM_ROW_13		0x1
#define DRAM_ROW_14		0x2
#define DRAM_ROW_15		0x3
#define DRAM_ROW_16		0x4

#define MODULE_ORG_OFFSET	0x7
#define DRAM_WIDTH_OFFSET	0x0
#define DRAM_WIDTH_BIT		0x7
#define DRAM_X4			0x0
#define DRAM_X8			0x1
#define DRAM_X16		0x2
#define DRAM_X32		0x3
#define MODULE_RANK_OFFSET	0x3
#define MODULE_RANK_BIT		0x7
#define MODULE_RANK_1		0x0
#define MODULE_RANK_2		0x1
#define MODULE_RANK_3		0x2
#define MODULE_RANK_4		0x3

#define MODULE_BUS_WIDTH_OFFSET	0x8
#define MODULE_WIDTH_OFFSET	0x0
#define MODULE_WIDTH_BIT	0x7
#define MODULE_WIDTH64		0x3
#define MODULE_WIDTH32		0x2
#define MODULE_WIDTH16		0x1
#define MODULE_WIDTH8		0x0

#define MODULE_ECC_OFFSET	0x3
#define MODULE_ECC_BIT		0x3
#define MODULE_ECC_INCLUDE	0x1

#define MIRROR_OFFSET		0x3f
#define MIRROR_SUPPORT		0x1

#define MC_INTERLEAVE_OFFSET	20	    //only work when NO_INTERLEAVE is not defined.(32 or above is not tested!!)
#define MC_INTERLEAVE_BIT	0x14ULL	//only work when NO_INTERLEAVE is not defined.(32 or above is not tested!!)
#define INTERLEAVE_10
#define ENABLE_DDR_CONFIG_SPACE		0x1
#define DISABLE_DDR_CONFIG_SPACE	0x0

#define BYPASS_CORE		0x0
#define BYPASS_NODE		0x0
#define BYPASS_L1		0x0

#define PLL_CHANG_COMMIT	0x1

#define BYPASS_REFIN		(0x1 << 0)
#define CORE_CLKSEL		0x1c
#define CORE_HSEL		0x0c
#define PLL_L1_ENA		(0x1 << 2)

// ddr config

struct mc_setting {
	struct udevice	*dev;
	struct ram_info	info;
	void __iomem	*soc_cfg_base;
	u32	mc_disable_offs;
	u32	mc_default_offs;
	u64     *params;

	u8	cs_map;
	u8	memsize;
	u8	slice_num;

	u8	sdram_type;
	u8	sdram_width;
	u8	sdram_banks;
	u8	sdram_rows;
	u8	sdram_cols;

	u8	dimm_type;
	u8	data_width;

	u8	ecc_en;
	bool	addr_mirror;
	bool	multi_channel;
	bool	reset_revert;

	u8	clk_latency;
	u8	rd_latency;
	u8	wr_latency;
	u8	cmd_timing_mode;
};

// node id
#define MC_NODE_ID_SHFIT	(0)
#define MC_NODE_ID_MASK		(0xf)
#define MC_SB_NODE_ID_SHFIT	(4)
#define MC_SB_NODE_ID_MASK	(0xf)

// mem controller select
#define MC_CTL_SEL_MC0 		(1 << 0)
#define MC_CTL_SEL_MC1		(1 << 1)

// sdram width
#define MC_SDRAM_WIDTH_X8	(0)
#define MC_SDRAM_WIDTH_X16	(1)

// cs map
#define MC_USE_CS_0		(1 << 0)
#define MC_USE_CS_1		(1 << 1)
#define MC_USE_CS_2		(1 << 2)
#define MC_USE_CS_3		(1 << 3)
#define MC_USE_CS_ALL		(MC_USE_CS_0 | MC_USE_CS_1 | MC_USE_CS_2 | MC_USE_CS_3)

// addr mirror
#define MC_ADDR_MIRROR_NO	(0)
#define MC_ADDR_MIRROR_YES	(1)

// sdram type
#define MC_SDRAM_TYPE_NODIMM	(0)
#define MC_SDRAM_TYPE_DDR2	(2)
#define MC_SDRAM_TYPE_DDR3	(3)

// sdram rows
#define MC_SDRAM_ROW(n)		(16 - (n))
#define MC_SDRAM_ROW_V2(n)	(15 - (n))

// sdram cols
#define MC_SDRAM_COL(n)		(16 - (n))

// sdram banks
#define MC_SDRAM_BANK_2		(1)
#define MC_SDRAM_BANK_4		(2)
#define MC_SDRAM_BANK_8		(3)

// dimm type
#define MC_DIMM_TYPE_RDIMM	(1)
#define MC_DIMM_TYPE_UDIMM	(2)
#define MC_DIMM_TYPE_SODIMM	(3)

// ecc
#define MC_ECC_EN_NO		(0)
#define MC_ECC_EN_YES		(1)

// data with
#define MC_DATA_WIDTH_16	DATA_WIDTH_16
#define MC_DATA_WIDTH_32	DATA_WIDTH_32
#define MC_DATA_WIDTH_64	DATA_WIDTH_64

// multi channel
#define MC_MUTLI_CHAN_YES	MULTI_CHANNEL_EN
#define MC_MUTLI_CHAN_NO	MULTI_CHANNEL_DIS

// cmd timing mode
#define MC_CMD_TIMING_MODE_1T	0
#define MC_CMD_TIMING_MODE_2T	1
#define MC_CMD_TIMING_MODE_3T	2

#if defined(CONFIG_SPL_BUILD) || !defined(CONFIG_SPL)
extern u64 ls_ddr_param[];
void mc_adjust_param(struct mc_setting *mc);
#endif

#endif /* __LOONGSON_DDR_H__ */