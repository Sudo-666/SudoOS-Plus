#include <common.h>
#include <dm.h>
#include <fdtdec.h>
#include <init.h>
#include <ram.h>
#include <asm/io.h>
#include <config.h>
#include "ls_ddr.h"

#ifdef DDR_DEBUG
#define mem_debug	printf
#else
#define mem_debug(s...)
#endif

#define DDR_CONF_SPACE		PHYS_TO_UNCACHED(0xff00000)
#define FACTMEMSIZE 		0x8

enum {
	MC_ID_2K500 = 0,
	MC_ID_2K1000,
};

#if defined(CONFIG_SPL_BUILD) || !defined(CONFIG_SPL)
static u8 uDimmOrder[9] = {0, 1, 2, 3, 4, 5, 6, 7, 8};
static u8 rDimmOrder[9] = {8, 3, 2, 1, 0, 4, 5, 6, 7};

static __inline__ void loop_delay(unsigned long long loops)
{
	volatile unsigned long long counts = loops;
	while (counts--);
}

static __inline__ void mc_write_reg8(struct mc_setting *mc, u32 offset, u8 data)
{
	*(volatile u8 *)(DDR_CONF_SPACE + offset) = data;
}

static __inline__ u8 mc_read_reg8(struct mc_setting *mc, u32 offset)
{
	return *(volatile u8 *)(DDR_CONF_SPACE + offset);
}

static __inline__ void mc_write_reg16(struct mc_setting *mc, u32 offset, u16 data)
{
	*(volatile u16 *)(DDR_CONF_SPACE + offset) = data;
}

static __inline__ u16 mc_read_reg16(struct mc_setting *mc, u32 offset)
{
	return *(volatile u16 *)(DDR_CONF_SPACE + offset);
}

static __inline__ void mc_write_reg32(struct mc_setting *mc, u32 offset, u32 data)
{
	*(volatile u32 *)(DDR_CONF_SPACE + offset) = data;
}

static __inline__ u32 mc_read_reg32(struct mc_setting *mc, u32 offset)
{
	return *(volatile u32 *)(DDR_CONF_SPACE + offset);
}

static __inline__ void mc_write_reg64(struct mc_setting *mc, u32 offset, u64 data)
{
	*(volatile u64 *)(DDR_CONF_SPACE + offset) = data;
}

static __inline__ u64 mc_read_reg64(struct mc_setting *mc, u32 offset)
{
	return *(volatile u64 *)(DDR_CONF_SPACE + offset);
}

static void wait_init_done(struct mc_setting *mc)
{
	u8 val;
	do {
		val = *(volatile u8 *)(DDR_CONF_SPACE + DRAM_INIT_OFFSET);
	} while (val == 0);
}

static void DLLWrDataSet(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
	if (val8 >= DLL_WRDQ_SUB){
		val8 -= DLL_WRDQ_SUB;
	}else{
		val8 = val8 + 0x80 - DLL_WRDQ_SUB;
	}
	mc_write_reg8(mc, DLL_WRDQ_OFFSET + SliceNum * 0x20, val8);
}

static void WrdqLtHalfSet(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	val8 = mc_read_reg8(mc, DLL_WRDQ_OFFSET + SliceNum * 0x20);
	if (val8 >= WRDQ_LT_HALF_STD){
		val8 = 0;
	}else{
		val8 = 1;
	}

	mc_write_reg8(mc, WRDQ_LT_HALF_OFFSET + SliceNum * 0x20, val8);
}


static void WrdqsLtHalfSet(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
	if (val8 >= WRDQS_LT_HALF_STD){
		val8 = 0;
	}else{
		val8 = 1;
	}

	mc_write_reg8(mc, WRDQS_LT_HALF_OFFSET + SliceNum * 0x20, val8);
}


static void WlvlGet0(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u8 i = 0;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do{
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	}while (val8 == 0);

	while ((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & WLVL_CHECK_BIT){
		val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
		val8 = (val8 + 1) & 0x7f;
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, val8);
#ifdef DQ_CHANGE_WITH_DQS
		DLLWrDataSet(mc, SliceNum);
		WrdqLtHalfSet(mc, SliceNum);
		WrdqsLtHalfSet(mc, SliceNum);
#endif
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		//mem_debug("write leveling: slice %d searching 0\n", SliceNum);

		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);
	}

	// filter the 0 to 1 glitch

	while (i < 0x20){
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);

		while ((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & WLVL_CHECK_BIT){
			val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
			val8 = (val8 + 1) & 0x7f;
			mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, val8);
#ifdef DQ_CHANGE_WITH_DQS
			DLLWrDataSet(mc, SliceNum);
			WrdqLtHalfSet(mc, SliceNum);
			WrdqsLtHalfSet(mc, SliceNum);
#endif
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
			do{
				val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
			}while (val8 == 0);

			i = 0;
		}
		i++;
	}
}

static void WlvlGet1(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u8 i = 0;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do{
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	}while (val8 == 0);

	while (!((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & WLVL_CHECK_BIT)){
		val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
		val8 = (val8 + 1) & 0x7f;
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, val8);
#ifdef DQ_CHANGE_WITH_DQS
		DLLWrDataSet(mc, SliceNum);
		WrdqLtHalfSet(mc, SliceNum);
		WrdqsLtHalfSet(mc, SliceNum);
#endif
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		// mem_debug("write leveling: slice %d searching 1\n", SliceNum);
		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);
	}

	// filter the 1 to 0 glitch
	while (i < WLVL_FILTER_LEN){
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);

		while (!((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & WLVL_CHECK_BIT)){
			val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum*0x20);
			val8 = (val8 + 1) & 0x7f;
			mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, val8);
#ifdef DQ_CHANGE_WITH_DQS
			DLLWrDataSet(mc, SliceNum);
			WrdqLtHalfSet(mc, SliceNum);
			WrdqsLtHalfSet(mc, SliceNum);
#endif
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
			do{
				val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
			}while (val8 == 0);

			i = 0;
		}
		i++;
	}

	//set back
	val8 = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);
	val8 = ((val8 > WLVL_FILTER_LEN) ? val8 - WLVL_FILTER_LEN : val8 + 0x80 - WLVL_FILTER_LEN) & 0x7f;
	mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, val8);
	DLLWrDataSet(mc, SliceNum);
	WrdqLtHalfSet(mc, SliceNum);
	WrdqsLtHalfSet(mc, SliceNum);
}

static void DLLAdj(struct mc_setting *mc, u32 SliceNum)
{
	u8 DllWrdqs;
	u8 DllWrdq;
	DllWrdqs = mc_read_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20);

	if (DllWrdqs >= DLL_WRDQ_SUB){
		DllWrdq = DllWrdqs - DLL_WRDQ_SUB;
	}else{
		DllWrdq = DllWrdqs + 0x80 - DLL_WRDQ_SUB;
	}

	if (DllWrdqs >= 0x0 && DllWrdqs < DLL_ADJ_RANGE){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, 0x8);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20, (0x08-DLL_WRDQ_SUB)&0x7f);
	}else if (DllWrdqs >= 0x40-DLL_ADJ_RANGE && DllWrdqs < 0x40){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, 0x38);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20, (0x38-DLL_WRDQ_SUB)&0x7f);
	}else if (DllWrdqs >= 0x40 && DllWrdqs < 0x40+DLL_ADJ_RANGE){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, 0x48);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20, (0x48-DLL_WRDQ_SUB)&0x7f);
	}else if (DllWrdqs >= 0x80-DLL_ADJ_RANGE && DllWrdqs <= 0x7f){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, 0x78);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20, (0x78-DLL_WRDQ_SUB)&0x7f);
	}

	if (DllWrdq >= 0x0 && DllWrdq < DLL_ADJ_RANGE){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, (0x08+DLL_WRDQ_SUB)&0x7f);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20,  0x08);
	}else if (DllWrdq >= 0x40-DLL_ADJ_RANGE && DllWrdq < 0x40){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, (0x38+DLL_WRDQ_SUB)&0x7f);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20,  0x38);
	}else if (DllWrdq >= 0x40 && DllWrdq < 0x40+DLL_ADJ_RANGE){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, (0x48+DLL_WRDQ_SUB)&0x7f);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20,  0x48);
	}else if (DllWrdq >= 0x80-DLL_ADJ_RANGE && DllWrdq <= 0x7f){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET + SliceNum * 0x20, (0x78+DLL_WRDQ_SUB)&0x7f);
		mc_write_reg8(mc, DLL_WRDQ_OFFSET  + SliceNum * 0x20,  0x78);
	}
}

#if 0
static void RdOeSub(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	val8 = mc_read_reg8(mc, RDOE_BEGIN_OFFSET+SliceNum*0x20);
	val8-= 1;
	val8 &= 0x3;
	mc_write_reg8(mc, RDOE_BEGIN_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDOE_END_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDODT_BEGIN_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDODT_END_OFFSET+SliceNum*0x20, val8);

	if (val8 == 0x3){
		mem_debug("ERROR: slice %01d RdOeSub underflow\n", SliceNum);
		//mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)-1);
	}
}

static void RdOeAdd(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	val8 = mc_read_reg8(mc, RDOE_BEGIN_OFFSET+SliceNum*0x20);
	val8+= 1;
	val8 &= 0x3;
	mc_write_reg8(mc, RDOE_BEGIN_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDOE_END_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDODT_BEGIN_OFFSET+SliceNum*0x20, val8);
	mc_write_reg8(mc, RDODT_END_OFFSET+SliceNum*0x20, val8);

	if (val8 == 0x0){
		mem_debug("ERROR: slice %01d RdOeAdd overflow\n", SliceNum);
		//mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)+1);
	}
}
#endif

static void DLLGateAdd(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u16 k;

	val8 = mc_read_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20);
	val8 = (val8 + 1) & 0x7f;
	mc_write_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20, val8);
#ifdef DDR_DEBUG
	mem_debug("dll_gate = %x\n", val8);
#endif
	if (val8 == 0x00) {
		//   RdOeAdd(mc, SliceNum);
		mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)+1);
#ifdef DDR_DEBUG
		mem_debug("ERROR: slice %01d RdOeAdd\n", SliceNum);
#endif
		for (k=0;k<512;k++){ //sync code
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
		}
	}
}

static void GlvlGet0(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u8 i = 0;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do{
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	}while (val8 == 0);

	while ((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & GLVL_CHECK_BIT){
		DLLGateAdd(mc, SliceNum);

		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		// mem_debug("gate leveling: slice %d searching 0\n", SliceNum);

		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);

	}

	// filter the 0 to 1 glitch
	while (i < 0x10){

		DLLGateAdd(mc, SliceNum);

		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);

		while ((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & GLVL_CHECK_BIT){
			DLLGateAdd(mc, SliceNum);

			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
			do{
				val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
			}while (val8 == 0);

			i = 0;
		}
		i++;
	}
	val8 = mc_read_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20);
#ifdef DDR_DEBUG
	mem_debug("gate leveling: slice %d found 0, dll_gate = 0x%02x\n", SliceNum, val8);
#endif
}

static void GlvlGet1(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u8 i = 0;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do {
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	} while (val8 == 0);

	while (!((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & GLVL_CHECK_BIT)){
		DLLGateAdd(mc, SliceNum);

		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

#ifdef DDR_DEBUG
		mem_debug("gate leveling: slice %d searching 1\n", SliceNum);
#endif

		do {
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		} while (val8 == 0);
	}

	// filter the 1 to 0 glitch

	while (i < GLVL_FILTER_LEN){
		DLLGateAdd(mc, SliceNum);

		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0

		do{
			val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
		}while (val8 == 0);

		while (!((mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum)) & GLVL_CHECK_BIT)){
			DLLGateAdd(mc, SliceNum);

#ifdef DDR_DEBUG
			mem_debug("gate leveling: slice %d filter 1 to 0\n", SliceNum);
#endif

			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
			do{
				val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
			}while (val8 == 0);

			i = 0;
		}
		i++;
	}

	//set back
	val8 = mc_read_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20);
	if (val8 >= GLVL_FILTER_LEN){
		val8 = (val8 - GLVL_FILTER_LEN) & 0x7f;
	}else{
		val8 = (val8 + 0x80 - GLVL_FILTER_LEN) & 0x7f;
		mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET) - 1);
		//for (k=0;k<512;k++){ //sync code
		//    mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
		//    mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
		//}
	}
	mc_write_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20, val8);

#ifdef DDR_DEBUG
	u8 tRDDATA = mc_read_reg8(mc, tRDDATA_OFFSET);
	mem_debug("gate leveling: slice %d found 1, dll_gate = 0x%02x, tRDDATA = 0x%02x\n", SliceNum, val8, tRDDATA);
#endif
}

#if 0
static void RddqsLtHalfSet(struct mc_setting *mc, u32 SliceNum)
{//TODO, add edge control
	u8 Val8_0;
	u8 Val8_1;
	u8 val8;
	Val8_0 = mc_read_reg8(mc, DLL_WRDQ_OFFSET + SliceNum * 0x20);
	Val8_1 = mc_read_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20);
	val8   = (Val8_0 + Val8_1) & 0x7f;
	if ((val8 >= RDDQS_LT_HALF_STD1) || (val8 < RDDQS_LT_HALF_STD2)){
		val8 = 1;
	}else{
		val8 = 0;
	}

	mc_write_reg8(mc, RDDQS_LT_HALF_OFFSET + SliceNum * 0x20, val8);
}
#endif

//11110000
//11111111
//00000000
//00001111

static void WrdqclkdelaySet(struct mc_setting *mc, u32 moduletype)
{
	//    u8 rdimm_has_found = 0;
	u32 i,j;
	if (moduletype == MC_DIMM_TYPE_UDIMM){
		//first set all wrdq_clkdelay to 0x0
		for (i = 0; i < 8+mc->ecc_en; i++){
			mc_write_reg8(mc, WRDQ_CLKDELAY_OFFSET+uDimmOrder[i] * 0x20, 0x0);
		}

		for (i = 0; i < 7+mc->ecc_en; i++){
			u8 last_value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + uDimmOrder[i] * 0x20);
			u8      value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + uDimmOrder[i+1] * 0x20);

			// mem_debug("slice %d: last_value = %x, value = %x\n", i, last_value, value);

			if (i == 0 && last_value == 1){
				// mem_debug("slice %d: SUB", i);
				u8 val8;
				val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
				val8 -= 1;
				mc_write_reg8(mc, tRDDATA_OFFSET, val8);

				val8 = mc_read_reg8(mc, tPHY_WRLAT_OFFSET);
				val8 -= 1;
				mc_write_reg8(mc, tPHY_WRLAT_OFFSET, val8);
			}

			if (last_value == 1 && value == 0){
				for (j=i+1;j<8+mc->ecc_en;j++){
					mc_write_reg8(mc, WRDQ_CLKDELAY_OFFSET+uDimmOrder[j] * 0x20, 0x1);
				}
				break;
			}
		}
	}else if (moduletype == MC_DIMM_TYPE_RDIMM) {
		//first set all wrdq_clkdelay to 0x0
		u8 already_sub = 0;
		for (i = 0; i < 8+mc->ecc_en; i++){
			mc_write_reg8(mc, WRDQ_CLKDELAY_OFFSET+rDimmOrder[i] * 0x20, 0x0);
		}

		for (i = 1-mc->ecc_en; i < 5; i++){
			u8 last_value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + rDimmOrder[i] * 0x20);
			u8      value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + rDimmOrder[i+1] * 0x20);
			// mem_debug("i = %x, last_value = %x, value = %x\n", i, last_value, value);
			if (i == 1-mc->ecc_en && last_value == 1){
				u8 val8;
				val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
				val8 -= 1;
				mc_write_reg8(mc, tRDDATA_OFFSET, val8);

				val8 = mc_read_reg8(mc, tPHY_WRLAT_OFFSET);
				val8 -= 1;
				mc_write_reg8(mc, tPHY_WRLAT_OFFSET, val8);
				already_sub = 1;
			}

			if (last_value == 1 && value == 0){
				for (j=i+1;j<5;j++){
					mc_write_reg8(mc, WRDQ_CLKDELAY_OFFSET+rDimmOrder[j] * 0x20, 0x1);
				}
				break;
			}
		}

		for (i = 5; i < 8; i++){

			u8 last_value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + rDimmOrder[i] * 0x20);
			u8      value = mc_read_reg8(mc, WRDQ_LT_HALF_OFFSET + rDimmOrder[i+1] * 0x20);
			// mem_debug("i = %x, last_value = %x, value = %x\n", i, last_value, value);

			if (i == 5 && last_value == 1){
				u8 val8;
				if (!already_sub){
					val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
					val8 -= 1;
					mc_write_reg8(mc, tRDDATA_OFFSET, val8);

					val8 = mc_read_reg8(mc, tPHY_WRLAT_OFFSET);
					val8 -= 1;
					mc_write_reg8(mc, tPHY_WRLAT_OFFSET, val8);
				}
			}

			if (last_value == 1 && value == 0){
				for (j=i+1;j<9;j++){
					mc_write_reg8(mc, WRDQ_CLKDELAY_OFFSET+rDimmOrder[j] * 0x20, 0x1);
					// mem_debug("j = %x\n", j);
				}
				break;
			}
		}
	}
}

static void DLLGateSub(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	u16 k;
	val8 = mc_read_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20);
	if (val8 >= DLL_GATE_SUB){
		val8 -= DLL_GATE_SUB;
	}else{
		val8 = val8 + 0x80 - DLL_GATE_SUB;

		//RdOeSub(mc, SliceNum);
		mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)-1);
		for (k=0;k<512;k++){ //sync code
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
			mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
		}
	}
	mc_write_reg8(mc, DLL_GATE_OFFSET + SliceNum * 0x20, val8);
}


static void RddqsCntCheck(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;

	u8 RddqsCnt1st = 0;
	u8 RddqsCnt2nd = 0;

	//first read;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do{
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	}while (val8 == 0);

	RddqsCnt1st = (mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum) >> 2) & 0x7;

	//second read;
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	do{
		val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
	}while (val8 == 0);

	RddqsCnt2nd = (mc_read_reg8(mc, LVL_RESP_OFFSET + SliceNum) >> 2) & 0x7;
#ifdef DDR_DEBUG
	if (((RddqsCnt2nd - RddqsCnt1st) & 0x7) != BURST_LENGTH/2){
		mem_debug("Slice %d: rddqs counter check error, counter value is: %d\n\n", SliceNum, ((RddqsCnt2nd - RddqsCnt1st) & 0x7));
	}else{
		mem_debug("Slice %d: rddqs counter check pass\n\n", SliceNum);
	}
#endif
}

static void RddqsPreambleCheck(struct mc_setting *mc, u32 SliceNum)
{
	u8 val8;
	//  u8 Max=0;
	u8 j;
	u16 k;
	//  u8 dll_gate[9];
	u8 dll_gate_ori;
	//  u8 rd_oe_begin_ori;
	//  u8 rd_oe_end_ori;
	u8 PreambleFound;
	u8 HasSub;

	//  for (i = 0; i < 8+mc->ecc_en; i++){
	//    dll_gate[i] = mc_read_reg8(mc, DLL_GATE_OFFSET+i*0x20);
	//  }

	//  //find the Max rd_oe_begin/end
	//  for (i = 0; i < 8+mc->ecc_en; i++){
	//    if (Max < mc_read_reg8(mc, RDOE_BEGIN_OFFSET+i*0x20)){
	//      Max = mc_read_reg8(mc, RDOE_BEGIN_OFFSET+i*0x20);
	//    }
	//  }
	//  mem_debug("Max = %d\n", Max);
	//
	//  for (i = 0; i < 8+mc->ecc_en; i++){
	//    val8 = mc_read_reg8(mc, RDOE_BEGIN_OFFSET+i*0x20);
	//    val8 = val8 + (3- Max);
	//	  mem_debug("rd_oe = %d\n", val8);
	//    mc_write_reg8(mc, RDOE_BEGIN_OFFSET+i*0x20, val8);
	//    mc_write_reg8(mc, RDOE_END_OFFSET+i*0x20, val8);
	//  }
	//
	//  val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
	//  val8 = val8 - (3 - Max);
	//  mc_write_reg8(mc, tRDDATA_OFFSET, val8);
	//  mem_debug("trdData = %d\n", val8);


	// save dll_gate
	dll_gate_ori = mc_read_reg8(mc, DLL_GATE_OFFSET+SliceNum*0x20);
	do {
		PreambleFound = 1;
		HasSub  = 0;
		for (j=0;j<PREAMBLE_LEN-0x10;j++){
			u8 Sample5 = 0;
			val8 = dll_gate_ori-0x10-j;
			val8 = val8 & 0x7f;
			mc_write_reg8(mc, DLL_GATE_OFFSET+SliceNum*0x20, val8);
#ifdef DDR_DEBUG
			mem_debug("slice %d dll_gate = 0x%x\n", SliceNum, val8);
#endif
			if (val8 == 0x7f){
				// RdOeSub(mc, SliceNum);
				mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)-1);
				HasSub = 1;

				for (k=0;k<512;k++){ //sync code
					mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
					mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
				}
			}

			for (k = 0; k < 5; k++) {
				mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
				mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
				do {
					val8 = mc_read_reg8(mc, LVL_DONE_OFFSET);
				} while (val8 == 0);

				Sample5 = (Sample5 << 1) | (mc_read_reg8(mc, LVL_RESP_OFFSET+SliceNum) & GLVL_CHECK_BIT & 0x1);
			}

#ifdef DDR_DEBUG
			mem_debug("slice %d glvl_resp = 0x%x\n", SliceNum, Sample5);
#endif
			if (Sample5 & 0x1f) {
				mem_debug("slice %d preamble check failed @ 0x%x\n", SliceNum, j);
				PreambleFound = 0;
				if (!HasSub){
					mc_write_reg8(mc, tRDDATA_OFFSET, mc_read_reg8(mc, tRDDATA_OFFSET)-1);
				}
				break;
			}
#ifdef DDR_DEBUG
			if (j == PREAMBLE_LEN - 0x10 - 1){
				mem_debug("slice %d preamble check pass\n", SliceNum);
			}
#endif
		}
	}while (!PreambleFound);

	// restore dll_gate
	//        mc_write_reg8(mc, DLL_GATE_OFFSET+SliceNum*0x20, dll_gate_ori);
	//        mc_write_reg8(mc, RDOE_BEGIN_OFFSET+SliceNum*0x20, rd_oe_begin_ori);
	//        mc_write_reg8(mc, RDOE_END_OFFSET+SliceNum*0x20, rd_oe_end_ori);
	//        for (k=0;k<512;k++){ //sync code
	//            mc_write_reg8(mc, LVL_REQ_OFFSET, 0x1);//first set to 0x1
	//            mc_write_reg8(mc, LVL_REQ_OFFSET, 0x0);//then set to 0x0
	//        }
	GlvlGet1(mc, SliceNum);
	DLLGateSub(mc, SliceNum);
}

//leveling
static void DDR3Leveling(struct mc_setting *mc, u8 NumSlice)
{
	u64 i = 0;
	//  u64 j = 0;
	//  u32 pm_dll_value;
	//  u32 RegAddr;
	u8  val8;
	//  u64 Val64;
	u8  tRddataOri;
	u8  tRDDATA[NumSlice];
	u8  Max=0;
	u8  Min=255;

	//ddr_dll_bypass(DLL_CK0_OFFSET);
	//ddr_dll_bypass(DLL_CK1_OFFSET);
	//ddr_dll_bypass(DLL_CK2_OFFSET);
	//ddr_dll_bypass(DLL_CK3_OFFSET);

	wait_init_done(mc);
#ifdef DDR_DEBUG
	mem_debug("write leveling begin\n");
#endif
	//set all DllWrdqs to 0x0
	for (i = 0; i < NumSlice; i++){
		mc_write_reg8(mc, DLL_WRDQS_OFFSET+i*0x20, 0x0);
	}
	//set leveling_mode
#ifdef DDR_DEBUG
	mem_debug("set leveling mode to be WRITE LEVELING\n");
#endif
	mc_write_reg8(mc, LVL_MODE_OFFSET, 0x1);
	do{
		val8 = mc_read_reg8(mc, LVL_READY_OFFSET);
	} while (val8 == 0);

#ifdef DDR_DEBUG
	mem_debug("write leveling ready\n");
#endif

	for (i = 0; i < NumSlice; i++){
		WlvlGet0(mc, i);
	}

	for (i = 0; i < NumSlice; i++){
		WlvlGet1(mc, i);
	}

	for (i = 0; i < NumSlice; i++){
		DLLAdj(mc, i);
	}

	WrdqclkdelaySet(mc, mc->dimm_type);

	mc_write_reg8(mc, LVL_MODE_OFFSET, 0x0);

#ifdef DDR_DEBUG
	u32 RegAddr;
	u64 Val64;
	mem_debug("the parameter after write leveling is:\n");
	for (i = 0; i < MC_REGS_COUNT; i++){
		RegAddr = 0x8 * i;
		Val64 = mc_read_reg64(mc, RegAddr);
		mem_debug("%03x: %016llx\n", RegAddr, Val64);
	}
	if (i%4 == 3){
		mem_debug("\n");
	}
#endif

	mc_write_reg8(mc, LVL_MODE_OFFSET, 0x0);

	//reset init_start
	mc_write_reg8(mc, INIT_START_OFFSET, 0x0);
	loop_delay(0x100);
	mc_write_reg8(mc, INIT_START_OFFSET, 0x1);
	wait_init_done(mc);

	//set all dll_gate to 0x0
	for (i = 0; i < NumSlice; i++){
		mc_write_reg8(mc, DLL_GATE_OFFSET+i*0x20, 0x0);
	}

	//set leveling_mode
#ifdef DDR_DEBUG
	mem_debug("set leveling mode to be Gate LEVELING\n");
#endif
	mc_write_reg8(mc, LVL_MODE_OFFSET, 0x2);

	do{
		val8 = mc_read_reg8(mc, LVL_READY_OFFSET);
	}while (val8 == 0);


	for (i = 0; i < NumSlice; i++){
		tRddataOri = mc_read_reg8(mc, tRDDATA_OFFSET);
		mc_write_reg8(mc, DLL_GATE_OFFSET+i*0x20, 0x0);
		GlvlGet0(mc, i);

		//reset init_start
		mc_write_reg8(mc, INIT_START_OFFSET, 0x0);
		loop_delay(0x100);
		mc_write_reg8(mc, INIT_START_OFFSET, 0x1);
		wait_init_done(mc);

		GlvlGet1(mc, i);
		// DLLGateSub(mc, i);
		RddqsPreambleCheck(mc, i);
		RddqsCntCheck(mc, i);
		tRDDATA[i] = mc_read_reg8(mc, tRDDATA_OFFSET);
		mc_write_reg8(mc, tRDDATA_OFFSET, tRddataOri);
	}

	// find the Max rd_oe_begin/end
	for (i = 0; i < NumSlice; i++){
		if (Max < tRDDATA[i]){
			Max = tRDDATA[i];
		}
		if (Min > tRDDATA[i]){
			Min = tRDDATA[i];
		}
	}

#ifdef DDR_DEBUG
	for (i = 0; i < NumSlice; i++){
		mem_debug("tRDDATA[%02lld] = 0x%02x\n", i, tRDDATA[i]);
	}
	mem_debug("Max = %02d, Min = %02d\n", Max, Min);
#endif

	if (Max-Min > 3){
		mem_debug("ERROR: read gate window difference is too large\n");
	}else{
		mc_write_reg8(mc, tRDDATA_OFFSET, Min + mc_read_reg8(mc, RDOE_BEGIN_OFFSET)); //here assume all slice has same rd_oe_begin/end
		for (i = 0; i < NumSlice; i++){
			val8 = tRDDATA[i] - Min;
			mc_write_reg8(mc, RDOE_BEGIN_OFFSET+i*0x20, val8);
			mc_write_reg8(mc, RDOE_END_OFFSET+i*0x20, val8);
			mc_write_reg8(mc, RDODT_BEGIN_OFFSET+i*0x20, val8);
			mc_write_reg8(mc, RDODT_END_OFFSET+i*0x20, val8);
		}
	}

	mc_write_reg8(mc, LVL_MODE_OFFSET, 0x0);

	//reset init_start
	mc_write_reg8(mc, INIT_START_OFFSET, 0x0);
	mc_write_reg8(mc, INIT_START_OFFSET, 0x1);
	wait_init_done(mc);

	mc_write_reg8(mc, 0x19, 0x1);
	mc_write_reg8(mc, 0x7, 0x0);
}

//reconfig
static void McParamReconfig(struct mc_setting *mc)
{
	u8 val8, PmCsMap, cs_num, memsize_cs;
	u64 Val64;
	int i;

	/*set tREFI*/
#ifndef TEMP_EXTREME
	mc_write_reg8(mc, 0x1c8 + 0x3, (DDR_FREQ / 10 * 78) >> 8);
#else
	mc_write_reg8(mc, 0x1c8 + 0x3, (DDR_FREQ / 10 * 78) >> 9);
#endif

	//reconfig cs map
	cs_num = 0;
	PmCsMap = 0;
	for (i = 0; i < 4; i++) {
		if ((mc->cs_map >> i) & 0x1) {
			PmCsMap = PmCsMap | i << (cs_num * 2);
			cs_num++;
		}
	}
	mc_write_reg8(mc, CS_MAP_OFFSET, PmCsMap);

#ifndef NO_AUTO_TRFC
	/*set tRFC*/
	memsize_cs = FACTMEMSIZE/cs_num;
	if (mc->sdram_width == MC_SDRAM_WIDTH_X16) {
		val8 = 0x1;
	} else {
		val8 = 0x2;
	}
	val8 = memsize_cs/val8;
	if (val8 < 1) {
		val8 = 0x9;
	} else if (val8 < 0x2) {
		val8 = 0xb;
	} else if (val8 < 0x4) {
		val8 = 0x10;
	} else if (val8 < 0x8) {
		val8 = 0x1a;
	} else if (val8 < 0x10) {
		val8 = 0x23;
	} else {
		mem_debug("memsize wrong\n");
	}
	mc_write_reg8(mc, 0x1c8 + 0x2, DDR_FREQ * val8 / 100);
#endif

	//for UDIMM 4cs,open 2T mode
	if (mc->dimm_type == MC_DIMM_TYPE_UDIMM && mc->cs_map == 0xf) {
		//add cmd_timing ,trddata and tphy_wrlat by one
		val8 = mc_read_reg8(mc, CMD_TIMING_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, CMD_TIMING_OFFSET, val8);

		val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, tRDDATA_OFFSET, val8);

		val8 = mc_read_reg8(mc, tPHY_WRLAT_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, tPHY_WRLAT_OFFSET, val8);
	}
	//rewrite eight_bank_mode
	//rewrite pm_bank_diff_0 and pm_bank
	//for UDIMM 4cs,open 2T mode
	if ((mc->dimm_type == MC_DIMM_TYPE_UDIMM) && mc->cs_map == 0xf) {
		//add cmd_timing ,trddata and tphy_wrlat by one
		val8 = mc_read_reg8(mc, CMD_TIMING_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, CMD_TIMING_OFFSET, val8);

		val8 = mc_read_reg8(mc, tRDDATA_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, tRDDATA_OFFSET, val8);

		val8 = mc_read_reg8(mc, tPHY_WRLAT_OFFSET);
		val8 += 1;
		mc_write_reg8(mc, tPHY_WRLAT_OFFSET, val8);
	}

	if (mc->sdram_banks == MC_SDRAM_BANK_8) {
		//rewrite eight_bank_mode
		//rewrite pm_bank_diff_0 and pm_bank
		mc_write_reg8(mc, BA_DIFF_OFFSET, 0x0);
		//for version 3, pm_bank not used, changed to cs_resync
		if (mc_read_reg8(mc, VERSION_OFFSET) != 3) {
			mc_write_reg8(mc, BANK_OFFSET, 0x7);
			//rewrite pm_addr_win
			mc_write_reg8(mc, ADDR_WIN_OFFSET, 0xf);
		}
	}

	//rewrite row_diff and column_diff
	mc_write_reg8(mc, COL_DIFF_OFFSET, mc->sdram_cols);
	mc_write_reg8(mc, ROW_DIFF_OFFSET, mc->sdram_rows);

	//mc_write_reg8(mc, BURST_LENGTH_OFFSET, BURST_LENGTH-1);

	//now only support 4R/2R/1R
	if (mc->cs_map == MC_USE_CS_ALL) {
		mc_write_reg8(mc, CS_DIFF_OFFSET, 0x0);
	} else if (mc->cs_map == 0x3 || mc->cs_map == 0xc || mc->cs_map == 0x5) {
		mc_write_reg8(mc, CS_DIFF_OFFSET, 0x1);
	} else if (mc->cs_map == 0x1 || mc->cs_map == 0x4) {
		mc_write_reg8(mc, CS_DIFF_OFFSET, 0x2);
	}

	mc_write_reg8(mc, CS_ENABLE_OFFSET, 0xf);
	mc_write_reg8(mc, CS_MRS_OFFSET, mc->cs_map);
	mc_write_reg8(mc, CS_ZQ_OFFSET, mc->cs_map);

	if (mc->cs_map & MC_USE_CS_0) {
		mc_write_reg8(mc, LVL_CS_OFFSET, 0x1);
	} else if (mc->cs_map & MC_USE_CS_2) {
		mc_write_reg8(mc, LVL_CS_OFFSET, 0x4);
	}

	if ((mc->dimm_type == MC_DIMM_TYPE_UDIMM) && mc->addr_mirror) {
		mc_write_reg8(mc, ADDR_MIRROR_OFFSET, 0xa);
	} else {
		mc_write_reg8(mc, ADDR_MIRROR_OFFSET, 0x0);
	}
#ifndef MANAUL_ODT_MAP
	//reconfig ODT map
	//set default first
	//clear map first
	Val64 = mc_read_reg64(mc, ODT_MAP_OFFSET);
	Val64 &= 0x0000ffff0000ffffULL;
	mc_write_reg64(mc, ODT_MAP_OFFSET, Val64);
	if (mc->sdram_type == MC_SDRAM_TYPE_DDR3) {
		Val64 = 0x8421000000000000ULL; //DDR3
	} else {
		Val64 = 0x8421000084210000ULL; //DDR2
	}
	Val64 |= mc_read_reg64(mc, ODT_MAP_OFFSET);
	mc_write_reg64(mc, ODT_MAP_OFFSET, Val64);
	//step 1: swap open wr odt if it's a Dual Rank DIMM
	//check cs_map[3]
	if (mc->cs_map & MC_USE_CS_3) {
		//slot 1 is a DR DIMM
		Val64 = mc_read_reg64(mc, ODT_MAP_OFFSET);
		Val64 &= 0x00ffffffffffffffULL;
		Val64 |= 0x4800000000000000ULL;
		mc_write_reg64(mc, ODT_MAP_OFFSET, Val64);
	}
	//check cs_map[1]
	if (mc->cs_map & MC_USE_CS_1) {
		//slot 0 is a DR DIMM
		Val64 = mc_read_reg64(mc, ODT_MAP_OFFSET);
		Val64 &= 0xff00ffffffffffffULL;
		Val64 |= 0x0012000000000000ULL;
		mc_write_reg64(mc, ODT_MAP_OFFSET, Val64);
	}

	//step 2: open extra RD/WR ODT CS if there is 2 DIMM
	//check CS[0] and CS[2]
	if (!(((mc->cs_map >> 0x2)^mc->cs_map) & 0x1)) {
		//2 DIMM: open the first rank of the non-target DIMM
		Val64 = mc_read_reg64(mc, ODT_MAP_OFFSET);
		Val64 |= 0x1144000011440000ULL;
		mc_write_reg64(mc, ODT_MAP_OFFSET, Val64);
	}
#endif

	if (mc->multi_channel) {
		//rewrite multi_channel mode
		Val64 = mc_read_reg64(mc, MULTI_CHANNEL_ADDR);
		Val64 &= ~(MULTI_CHANNEL_MASK << MULTI_CHANNEL_OFFSET);
		Val64 |= (MULTI_CHANNEL_EN << MULTI_CHANNEL_OFFSET);
		mc_write_reg64(mc, MULTI_CHANNEL_ADDR, Val64);
	} else {
		//set data bus width
		Val64 = mc_read_reg64(mc, DATA_WIDTH_ADDR);
		Val64 &= ~(DATA_WIDTH_MASK << DATA_WIDTH_OFFSET);
		Val64 |= (mc->data_width << DATA_WIDTH_OFFSET);
		mc_write_reg64(mc, DATA_WIDTH_ADDR, Val64);
	}

	for (i = 0; i < 4; i++) {
		//rewrite pm_addr_win(data width)
		val8 = mc_read_reg8(mc, ADDR_WIN_OFFSET + i * 0x8);
		val8 &= ~0xf;
		if (mc->sdram_banks == MC_SDRAM_BANK_8)
			val8 |= 0xc;
		else if (mc->sdram_banks == MC_SDRAM_BANK_4)
			val8 |= 0x8;
		else
			val8 |= 0x4;

		if (mc->data_width == MC_DATA_WIDTH_64)
			val8 |= 0x3;
		else if (mc->data_width == MC_DATA_WIDTH_32)
			val8 |= 0x2;
		else if (mc->data_width == MC_DATA_WIDTH_16)
			val8 |= 0x1;
		else
			val8 |= 0x3;
		mc_write_reg8(mc, ADDR_WIN_OFFSET + i * 0x8, val8);
	}

	//disable ECC module here for leveling, ECC will be enabled later
	Val64 = mc_read_reg64(mc, ECC_ENABLE_ADDR);
	Val64 &= ~(0x7ULL << ECC_ENABLE_OFFSET);
	mc_write_reg64(mc, ECC_ENABLE_ADDR, Val64);
}

static void McConfSpace(struct mc_setting *mc, int enable)
{
	volatile u32 *pChipConfigBase =(u32 *)(mc->soc_cfg_base);

	if (enable) {
		writel(readl(pChipConfigBase) & (~(1 << mc->mc_disable_offs)), pChipConfigBase);
	} else {
		writel(readl(pChipConfigBase) | (1 << mc->mc_disable_offs), pChipConfigBase);
		loop_delay(0x100);
		writel(readl(pChipConfigBase) & (~(1 << mc->mc_default_offs)), pChipConfigBase);
	}
}

void mc_init(struct mc_setting *mc)
{
	volatile u64 *pMcRegsBase = 0;
	volatile u8 *pMcRegsBase8 = 0;
	u8 val8, Data8 = 0x33;
	u64 val64;
	int i;

	mc_adjust_param(mc);

	// mem_debug("Enable register space of MEMORY\n");
	McConfSpace(mc, ENABLE_DDR_CONFIG_SPACE);

	for (i = 0; i < MC_REGS_COUNT; i++) {
		pMcRegsBase = (u64 *)(DDR_CONF_SPACE + (i << 3));
		*pMcRegsBase = mc->params[i];
	}

#ifdef DDR_DEBUG
	mem_debug("The MC parameter before reconfig is:\n");
	for (i = 0; i < MC_REGS_COUNT; i++){
		pMcRegsBase = (u64 *)(DDR_CONF_SPACE + (i << 3));
		u64 Data = *pMcRegsBase;
		mem_debug("%p: %016llx\n", pMcRegsBase, Data);
	}
#endif

	McParamReconfig(mc);

#ifdef DDR_DEBUG
	mem_debug("The MC parameter after reconfig is:\n");
	for (i = 0; i < MC_REGS_COUNT; i++){
		pMcRegsBase = (u64 *)(DDR_CONF_SPACE + (i << 3));
		u64 Data = *pMcRegsBase;
		mem_debug("%p: %016llx\n", pMcRegsBase, Data);
	}
#endif

	/***** set start to 1,start to initialize SDRAM *****/
	pMcRegsBase8 = (u8 *)(DDR_CONF_SPACE + INIT_START_OFFSET);
	val8 = *pMcRegsBase8;
	val8 |= 0x1;
	*pMcRegsBase8 = val8;
	loop_delay(0x100);
	while (val8 != Data8){
		/* wait initialization complete */
		pMcRegsBase8 = (u8 *)(DDR_CONF_SPACE + DRAM_INIT_OFFSET);
		//    Val >>= DRAM_INIT_OFFSET;
		val8 = *pMcRegsBase8;
		val8 &= 0x1;

		pMcRegsBase8 = (u8 *)(DDR_CONF_SPACE + CS_ENABLE_OFFSET);
		Data8 = *pMcRegsBase8;
		Data8 &= 0x1;
		/* Just for delay */
		loop_delay(0x100000);
	}

#ifdef DDR_DEBUG
	mem_debug("The MC parameter before leveling is:\n");
	for (i = 0; i < MC_REGS_COUNT; i++){
		pMcRegsBase = (u64 *)(DDR_CONF_SPACE + (i << 3));
		u64 Data = *pMcRegsBase;
		mem_debug("%p: %016llx\n", pMcRegsBase, Data);
	}
#endif

	DDR3Leveling(mc, mc->slice_num + mc->ecc_en);

#ifdef DDR_DEBUG
	mem_debug("The MC parameter after leveling is:\n");
	for (i = 0; i < MC_REGS_COUNT; i++) {
		pMcRegsBase = (u64 *)(DDR_CONF_SPACE + (i << 3));
		u64 Data = *pMcRegsBase;
		mem_debug("%p: %016llx\n", pMcRegsBase, Data);
		if (i % 4 == 3){
			mem_debug("\n");
		}
	}
#endif

	if (mc->ecc_en) {
		val64 = mc_read_reg64(mc, ECC_ENABLE_ADDR);
		val64 &= ~(0x7ULL << ECC_ENABLE_OFFSET);
		val64 |= (MC_ECC_EN_YES << ECC_ENABLE_OFFSET);
		mc_write_reg64(mc, ECC_ENABLE_ADDR, val64);
	}

	McConfSpace(mc, DISABLE_DDR_CONFIG_SPACE);

#ifdef DDR_DEBUG
	u64 memaddr = PHYS_TO_UNCACHED(0x00000000);	// DDR Low 256M space
	u64 test_val[8] = { 0x5555555555555555, 0xaaaaaaaaaaaaaaaa,
		0x3333333333333333, 0xcccccccccccccccc, 0x7777777777777777,
		0x8888888888888888, 0x1111111111111111, 0xeeeeeeeeeeeeeeee
	};

	for (i = 0; i < 8; i++) {
		writeq(test_val[i], (volatile u64 *)(memaddr + (i<<3)));
		if (readq((volatile u64 *)(memaddr + (i << 3))) != test_val[i]) {
			mem_debug("DDR init failed, uncached low addr test failed\n");
			goto ddr_out;
		}
	}

	memaddr = PHYS_TO_UNCACHED(0x80000000);	// DDR 2G space
	for (i = 0; i < 8; i++) {
		writeq(test_val[i], (volatile u64 *)(memaddr + (i<<3)));
		if (readq((volatile u64 *)(memaddr + (i << 3))) != test_val[i]) {
			mem_debug("DDR init failed, uncached high addr test failed\n");
			goto ddr_out;
		}
	}

	memaddr = PHYS_TO_CACHED(0x00000000);	// DDR Low 256M space
	for (i = 0; i < 8; i++) {
		writeq(test_val[i], (volatile u64 *)(memaddr + (i<<3)));
		if (readq((volatile u64 *)(memaddr + (i << 3))) != test_val[i]) {
			mem_debug("DDR init failed, cached low addr test failed\n");
			goto ddr_out;
		}
	}

	memaddr = PHYS_TO_CACHED(0x80000000);	// DDR 2G space
	for (i = 0; i < 8; i++) {
		writeq(test_val[i], (volatile u64 *)(memaddr + (i<<3)));
		if (readq((volatile u64 *)(memaddr + (i << 3))) != test_val[i]) {
			mem_debug("DDR init failed, uncache high addr test failed\n");
			goto ddr_out;
		}
	}

	mem_debug("DDR test OK\n");
ddr_out:
#endif

	return;
}
#endif

static int ls_ddr_parse_dt(struct udevice *dev)
{
	struct mc_setting *mc = dev_get_priv(dev);
	const char *str;
	u32 val;
	int mc_id, ret;

	mc_id = dev->driver_data;

	ret = dev_read_u32(dev, "soc-config-reg", &val);
	if (ret) {
		debug("ddr: get soc-config-reg failed!\n");
		return ret;
	}
	mc->soc_cfg_base = (void*)PHYS_TO_UNCACHED(val);

	ret = dev_read_u32(dev, "mc-disable-offs", &val);
	if (ret) {
		debug("ddr: get mc-disable-offs failed!\n");
		return ret;
	}
	mc->mc_disable_offs = val;

	ret = dev_read_u32(dev, "mc-default-offs", &val);
	if (ret) {
		debug("ddr: get mc-default-offs failed!\n");
		return ret;
	}
	mc->mc_default_offs = val;

	str = dev_read_string(dev, "sdram-type");
	if (str && !strcmp(str, "ddr2")) {
		mc->sdram_type = MC_SDRAM_TYPE_DDR2;
	} else if (str && !strcmp(str, "ddr3")) {
		mc->sdram_type = MC_SDRAM_TYPE_DDR3;
	} else {
		mc->sdram_type = MC_SDRAM_TYPE_DDR3;
	}

	str = dev_read_string(dev, "dimm-type");
	if (str && !strcmp(str, "rdimm")) {
		mc->dimm_type = MC_DIMM_TYPE_RDIMM;
	} else if (str && !strcmp(str, "sodimm")) {
		mc->dimm_type = MC_DIMM_TYPE_SODIMM;
	} else {
		mc->dimm_type = MC_DIMM_TYPE_UDIMM;
	}

	ret = dev_read_u32(dev, "cs-map", &val);
	if (ret) {
		debug("ddr: get cs-map failed!\n");
		return ret;
	}
	mc->cs_map = val;

	ret = dev_read_u32(dev, "data-width", &val);
	if (ret) {
		debug("ddr: get data-width failed!\n");
		return ret;
	}
	switch (val)
	{
	case 16:
		mc->data_width = MC_DATA_WIDTH_16;
		mc->slice_num = 2;
		break;
	case 32:
		mc->data_width = MC_DATA_WIDTH_32;
		mc->slice_num = 4;
		break;
	case 64:
		mc->data_width = MC_DATA_WIDTH_64;
		mc->slice_num = 8;
		break;
	default:
		debug("ddr: data width error!\n");
		return -EINVAL;
	}

	ret = dev_read_u32(dev, "sdram-width", &val);
	if (ret) {
		debug("ddr: get sdram-width failed!\n");
		return ret;
	}
	switch (val)
	{
	case 8:
		mc->sdram_width = MC_SDRAM_WIDTH_X8;
		break;
	case 16:
		mc->sdram_width = MC_SDRAM_WIDTH_X16;
		break;
	default:
		debug("ddr: sdram width error!\n");
		return -EINVAL;
	}

	ret = dev_read_u32(dev, "sdram-banks", &val);
	if (ret) {
		debug("ddr: get sdram-banks failed!\n");
		return ret;
	}
	switch (val)
	{
	case 2:
		mc->sdram_banks = MC_SDRAM_BANK_2;
		break;
	case 4:
		mc->sdram_banks = MC_SDRAM_BANK_4;
		break;
	case 8:
		mc->sdram_banks = MC_SDRAM_BANK_8;
		break;
	default:
		debug("ddr: sdram banks error!\n");
		return -EINVAL;
	}

	ret = dev_read_u32(dev, "sdram-rows", &val);
	if (ret) {
		debug("ddr: get sdram-rows failed!\n");
		return ret;
	}
	if (mc_id == MC_ID_2K1000) {
		mc->sdram_rows = MC_SDRAM_ROW_V2(val);
	} else {
		mc->sdram_rows = MC_SDRAM_ROW(val);
	}

	ret = dev_read_u32(dev, "sdram-cols", &val);
	if (ret) {
		debug("ddr: get sdram-cols failed!\n");
		return ret;
	}
	mc->sdram_cols = MC_SDRAM_COL(val);

	if (dev_read_bool(dev, "ecc-enable"))
		mc->ecc_en = MC_ECC_EN_YES;

	if (dev_read_bool(dev, "addr-mirror"))
		mc->addr_mirror = true;
	
	if (dev_read_bool(dev, "multi-channel"))
		mc->multi_channel = true;

	if (dev_read_bool(dev, "reset-revert"))
		mc->reset_revert = true;

	ret = dev_read_u32(dev, "clk-latency", &val);
	if (!ret)
		mc->clk_latency = val;
	
	ret = dev_read_u32(dev, "rd-latency", &val);
	if (!ret)
		mc->rd_latency = val;
	
	ret = dev_read_u32(dev, "wr-latency", &val);
	if (!ret)
		mc->wr_latency = val;

	ret = dev_read_u32(dev, "cmd-timing-mode", &val);
	if (!ret) {
		if (val == 0)
			mc->cmd_timing_mode = MC_CMD_TIMING_MODE_1T;
		else if (val == 1)
			mc->cmd_timing_mode = MC_CMD_TIMING_MODE_2T;
		else if (val == 2)
			mc->cmd_timing_mode = MC_CMD_TIMING_MODE_3T;
		else
			mc->cmd_timing_mode = MC_CMD_TIMING_MODE_2T;
	}

	return 0;
}

static int ls_ddr_probe(struct udevice *dev)
{
	struct mc_setting *mc = dev_get_priv(dev);

	mc->sdram_type = MC_SDRAM_TYPE_DDR3;
	mc->ecc_en = MC_ECC_EN_NO;
	mc->addr_mirror = false;
	mc->multi_channel = false;
	mc->reset_revert = false;
	mc->clk_latency = 0x40;
	mc->rd_latency = 0x9;
	mc->wr_latency = 0x4;
	mc->cmd_timing_mode = MC_CMD_TIMING_MODE_2T;

	if (ls_ddr_parse_dt(dev)) {
		debug("ddr: parse device tree failed!\n");
		return -EINVAL;
	}

	mc->info.base = PHYS_TO_CACHED(0x80000000);
	mc->info.size = (1 << mc->sdram_rows) * (1 << mc->sdram_cols)
			* mc->sdram_banks * (mc->sdram_width / 8)
			* (mc->data_width / 8);

#if defined(CONFIG_SPL_BUILD) || !defined(CONFIG_SPL)
	mc->params = ls_ddr_param;
	mc_init(mc);
#endif

	return 0;
}

static int ls_ddr_get_info(struct udevice *dev, struct ram_info *info)
{
	struct mc_setting *priv = dev_get_priv(dev);

	*info = priv->info;

	return 0;
}

static struct ram_ops ls_ddr_ops = {
	.get_info = ls_ddr_get_info,
};

static const struct udevice_id ls_ddr_ids[] = {
	{ .compatible = "loongson,ls2k500-ddr", .data = MC_ID_2K500 },
	{ .compatible = "loongson,ls2k1000-ddr", .data = MC_ID_2K1000 },
	{ }
};

U_BOOT_DRIVER(ls_ddr) = {
	.name = "ls_ddr",
	.id = UCLASS_RAM,
	.of_match = ls_ddr_ids,
	.ops = &ls_ddr_ops,
	.probe = ls_ddr_probe,
	.priv_auto = sizeof(struct mc_setting),
};