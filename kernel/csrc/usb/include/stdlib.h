/* freestanding <stdlib.h> shim（SudoOS 内核 C 胶水）。
 *
 * CherryUSB usb_mem.h 把 usb_malloc/usb_free 宏映射到 malloc/free；
 * 这两个符号由 usb_glue_ls2k1000.c 包装到内核分配器
 * (sudoos_usb_alloc/sudoos_usb_free)。
 */
#ifndef _SUDOOS_STDLIB_H
#define _SUDOOS_STDLIB_H

#include <stddef.h>

void *malloc(size_t n);
void free(void *ptr);

#endif /* _SUDOOS_STDLIB_H */
