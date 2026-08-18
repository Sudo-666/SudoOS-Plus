/*
 * kernel/csrc/usb/usb_glue_ls2k1000.c
 *
 * LS2K1000 USB（CherryUSB）平台胶水。
 *
 * M0：构建路径探针 sudoos_usb_glue_probe。
 * M1：CherryUSB 宿主初始化 + 最小运行时支撑（malloc/free 包装、mini
 *     vsnprintf/snprintf/printf、CDC/HID 类驱动桩、msc_test 桩）。
 *     EHCI 描述符/缓冲的 cache 一致性 M1 不处理（工具链无法汇编 cache
 *     指令，方案见 docs/decisions/ADR-001），留到 M2 真机确认。
 *
 * EHCI 基址经 uncached DMW 窗口（0x8000_0000_0000_0000 | phys）访问，
 * 与 LS2K1000 UART 驱动同款模式（arch/loongarch64/platform/ls2k1000）。
 * PHY/时钟/复位由 U-Boot 引导阶段初始化，本驱动直接复用。
 */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include "usbh_core.h"
#include "usb_hc.h"

/* 探针哨兵：0x2A4A0001 = "LSU-B1"。真机串口应打印此值。 */
unsigned int sudoos_usb_glue_probe(void)
{
    return 0x2A4A0001u;
}

/* ---- CherryUSB usb_mem.h 的 usb_malloc/usb_free 宏映射到 malloc/free ---- */
extern void *sudoos_usb_alloc(unsigned long size);
extern void sudoos_usb_free(void *ptr);

void *malloc(size_t n)
{
    return sudoos_usb_alloc(n);
}

void free(void *ptr)
{
    sudoos_usb_free(ptr);
}

/* ---- 串口输出（Rust 侧导出，见 kernel/src/cusb.rs）---- */
extern void sudoos_usb_log_str(const char *s);

/* ---- mini vsnprintf：CherryUSB 用它生成 "/dev/sd%c" 等 devname，
 *      printf 走 usbh_print_hubport_info 直接调用。
 *      支持 %d %u %x %X %c %s %p %% 与最小 %0Nd 宽度/零填充。 ---- */
static void fmt_unsigned(char *out, size_t out_size, size_t *n,
                         unsigned long v, int width, int zero,
                         unsigned base, int upper)
{
    char digits[32];
    int len = 0;
    const char *xdig = upper ? "0123456789ABCDEF" : "0123456789abcdef";

    if (v == 0) {
        digits[len++] = '0';
    }
    while (v != 0) {
        digits[len++] = xdig[v % base];
        v /= base;
    }
    while (len < width && *n + 1 < out_size) {
        out[(*n)++] = zero ? '0' : ' ';
        width--;
    }
    for (int i = len - 1; i >= 0 && *n + 1 < out_size; i--) {
        out[(*n)++] = digits[i];
    }
}

static void fmt_signed(char *out, size_t out_size, size_t *n,
                       long v, int width, int zero)
{
    if (v < 0) {
        if (*n + 1 < out_size) {
            out[(*n)++] = '-';
        }
        fmt_unsigned(out, out_size, n, (unsigned long)(-v), width - 1, zero, 10, 0);
    } else {
        fmt_unsigned(out, out_size, n, (unsigned long)v, width, zero, 10, 0);
    }
}

static int sudoos_vsnprintf(char *buf, size_t size, const char *fmt, va_list ap)
{
    size_t n = 0;
    const char *p;

    if (size == 0) {
        return 0;
    }
    for (p = fmt; *p != '\0' && n + 1 < size; p++) {
        int width = 0;
        int zero = 0;
        if (*p != '%') {
            buf[n++] = *p;
            continue;
        }
        p++;
        if (*p == '0') {
            zero = 1;
            p++;
        }
        while (*p >= '0' && *p <= '9') {
            width = width * 10 + (*p - '0');
            p++;
        }
        switch (*p) {
        case 'd':
        case 'i':
            fmt_signed(buf, size, &n, va_arg(ap, int), width, zero);
            break;
        case 'u':
            fmt_unsigned(buf, size, &n, va_arg(ap, unsigned int), width, zero, 10, 0);
            break;
        case 'x':
            fmt_unsigned(buf, size, &n, va_arg(ap, unsigned int), width, zero, 16, 0);
            break;
        case 'X':
            fmt_unsigned(buf, size, &n, va_arg(ap, unsigned int), width, zero, 16, 1);
            break;
        case 'p':
            if (n + 3 < size) {
                buf[n++] = '0';
                buf[n++] = 'x';
            }
            fmt_unsigned(buf, size, &n, (unsigned long)va_arg(ap, void *), 0, 1, 16, 0);
            break;
        case 'c': {
            char c = (char)va_arg(ap, int);
            if (n + 1 < size) {
                buf[n++] = c;
            }
            break;
        }
        case 's': {
            const char *s = va_arg(ap, const char *);
            if (s == NULL) {
                s = "(null)";
            }
            while (*s != '\0' && n + 1 < size) {
                buf[n++] = *s++;
            }
            break;
        }
        case '%':
            buf[n++] = '%';
            break;
        default:
            buf[n++] = '%';
            if (n + 1 < size) {
                buf[n++] = *p;
            }
            break;
        }
    }
    buf[n] = '\0';
    return (int)n;
}

int snprintf(char *buf, size_t size, const char *fmt, ...)
{
    va_list ap;
    int ret;

    va_start(ap, fmt);
    ret = sudoos_vsnprintf(buf, size, fmt, ap);
    va_end(ap);
    return ret;
}

int printf(const char *fmt, ...)
{
    char buf[256];
    va_list ap;

    va_start(ap, fmt);
    sudoos_vsnprintf(buf, sizeof(buf), fmt, ap);
    va_end(ap);
    sudoos_usb_log_str(buf);
    return 0;
}

/* ---- CDC/HID 类驱动桩 ----
 *
 * usbh_core.c 的 class_info_table 无条件引用这两个驱动，但本配置只挂
 * MSC。若真插入 CDC/HID 设备，connect 为 NULL 会失败（不会崩溃）。
 */
const struct usbh_class_driver cdc_acm_class_driver = {
    "cdc_acm",
    NULL,
    NULL,
};

const struct usbh_class_driver hid_class_driver = {
    "hid",
    NULL,
    NULL,
};

/* ---- msc_test 桩：vendored 快照里 usbh_msc_connect 的调试残留 ---- */
int msc_test(void)
{
    return 0;
}

/* ---- CherryUSB 宿主初始化（M1）----
 *
 * usb_hc_init()：     EHCI HCD 初始化（经 uncached DMW 访问 MMIO）。
 * usbh_initialize()： 宿主栈（workqueue + psc 线程，M1 线程为桩）。
 * 返回 0 成功；负值失败。
 */
int sudoos_usb_init(void)
{
    if (usb_hc_init() != 0) {
        return -1;
    }
    if (usbh_initialize() != 0) {
        return -2;
    }
    return 0;
}
