/* freestanding <stdio.h> shim（SudoOS 内核 C 胶水）。
 *
 * 仅声明 printf：USB_DBG_LEVEL=-1 时 CherryUSB 日志宏全为 no-op，
 * printf 不被引用，故无需实现。M2 如需真机枚举日志再补 mini printf。
 */
#ifndef _SUDOOS_STDIO_H
#define _SUDOOS_STDIO_H

#include <stddef.h>

/* printf 仅声明：USB_DBG_LEVEL=-1 时 CherryUSB 日志宏全为 no-op，
 * printf 不被引用，故无需实现。 */
int printf(const char *fmt, ...);

/* snprintf 由 usb_glue_ls2k1000.c 实现（mini 版本，无 libc）。 */
int snprintf(char *buf, size_t size, const char *fmt, ...);

#endif /* _SUDOOS_STDIO_H */
