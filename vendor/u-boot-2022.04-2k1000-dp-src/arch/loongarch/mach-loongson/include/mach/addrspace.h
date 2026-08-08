#ifndef __MACH_LS_CPU_H
#define __MACH_LS_CPU_H

#include <linux/sizes.h>

#define MEM_WIN_BASE           (0x00000000)
#define MEM_WIN_SIZE           (SZ_256M)

#define HIGH_MEM_WIN_BASE      (0x80000000)
#define HIGH_MEM_WIN_SIZE      (SZ_2G)

#endif /* __MACH_LS_CPU_H */