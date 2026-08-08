#ifndef __MACH_LA_LOONGSON_H
#define __MACH_LA_LOONGSON_H

#include <config.h>

#ifdef CONFIG_SOC_LS2K500
#include "ls2k500/ls2k500.h"
#elif defined(CONFIG_SOC_LS2K1000)
#include "ls2k1000/ls2k1000.h"
#endif

#define LS_PM_SOC_REG				(LS_ACPI_REG_BASE + 0x00)
#define LS_PM_RESUME_REG			(LS_ACPI_REG_BASE + 0x04)
#define LS_PM_RTC_REG				(LS_ACPI_REG_BASE + 0x08)
#define LS_PM1_STS_REG				(LS_ACPI_REG_BASE + 0x0c)
#define LS_PM1_EN_REG				(LS_ACPI_REG_BASE + 0x10)
#define LS_PM1_CNT_REG				(LS_ACPI_REG_BASE + 0x14)
#define LS_PM1_TMR_REG				(LS_ACPI_REG_BASE + 0x18)
#define LS_P_CNT_REG				(LS_ACPI_REG_BASE + 0x1c)
#define LS_P_LVL2_REG				(LS_ACPI_REG_BASE + 0x20)
#define LS_P_LVL3_REG				(LS_ACPI_REG_BASE + 0x24)
#define LS_GPE0_STS_REG				(LS_ACPI_REG_BASE + 0x28)
#define LS_GPE0_EN_REG				(LS_ACPI_REG_BASE + 0x2c)
#define LS_RST_CNT_REG				(LS_ACPI_REG_BASE + 0x30)
#define LS_WD_SET_REG				(LS_ACPI_REG_BASE + 0x34)
#define LS_WD_TIMER_REG				(LS_ACPI_REG_BASE + 0x38)
#define LS_DVFS_CNT_REG				(LS_ACPI_REG_BASE + 0x3c)
#define LS_DVFS_STS_REG				(LS_ACPI_REG_BASE + 0x40)
#define LS_MS_CNT_REG				(LS_ACPI_REG_BASE + 0x44)
#define LS_MS_THT_REG				(LS_ACPI_REG_BASE + 0x48)
#define	LS_THSENS_CNT_REG			(LS_ACPI_REG_BASE + 0x4c)
#define LS_GEN_RTC1_REG				(LS_ACPI_REG_BASE + 0x50)
#define LS_GEN_RTC2_REG				(LS_ACPI_REG_BASE + 0x54)

#endif /* __MACH_LA_LOONGSON_H */