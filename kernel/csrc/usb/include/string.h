/* freestanding <string.h> shim（SudoOS 内核 C 胶水）。
 *
 * CherryUSB 拉 <string.h> 会命中交叉 libc 头，而 -mabi=lp64s 与 glibc
 * 的 stubs-lp64d 不匹配。这里只声明符号，实现由 Rust compiler-builtins
 * (mem) 提供；配合 build.rs 的 -nostdinc 生效。
 */
#ifndef _SUDOOS_STRING_H
#define _SUDOOS_STRING_H

#include <stddef.h>

void *memcpy(void *dst, const void *src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *s, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);
size_t strlen(const char *s);
int strcmp(const char *a, const char *b);
int strncmp(const char *a, const char *b, size_t n);

#endif /* _SUDOOS_STRING_H */
