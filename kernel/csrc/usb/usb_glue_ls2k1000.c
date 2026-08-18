/*
 * kernel/csrc/usb/usb_glue_ls2k1000.c
 *
 * LS2K1000 USB（CherryUSB）平台胶水。
 *
 * M0：构建路径探针 sudoos_usb_glue_probe。
 * M1：CherryUSB 宿主初始化 + 最小运行时支撑（malloc/free 包装、mini
 *     vsnprintf/snprintf/printf、CDC/HID 类驱动桩、msc_test 桩）。
 * M2：真实线程 + WaitQueue 信号量（usb_osal_sudoos.c）接入后，本文件：
 *     - 单次初始化：只调 `usbh_initialize()`，`usb_hc_init()` 由 psc
 *       线程自行调用（移除 M1 的显式调用，避免双初始化）；
 *     - 覆盖 `usb_hc_low_level_init()`：复用 U-Boot 时钟/PHY 状态，
 *       打印 EHCI 寄存器供真机 bring-up 诊断；
 *     - 覆盖 `usbh_get_port_speed()`：读 PORTSC bits[13:10]；
 *     - `sudoos_usb_wait_device()`：轮询 root hub 端口识别 VID:PID。
 *
 * EHCI 基址经 uncached DMW 窗口（0x8000_0000_0000_0000 | phys）访问，
 * 与 LS2K1000 UART 驱动同款模式（arch/loongarch64/platform/ls2k1000）。
 * 传输完成由 kernel/src/cusb.rs 的 usb_poller 线程轮询
 * `usb_ehci_interrupt()` 驱动（2K1000 无外设中断基础设施，见 ADR-001）。
 */
#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include "usbh_core.h"
#include "usb_hc.h"
#include "usb_ehci.h"

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

/* ---- 时钟/延时（Rust 导出；sleep_ms 需在 C 线程上下文调用）---- */
extern void sudoos_usb_sleep_ms(unsigned int ms);

/* ---- EHCI 低层初始化（覆盖 __WEAK 默认）----
 *
 * 2K1000 的 USB PHY/时钟/复位由 U-Boot 引导阶段初始化（fatload usb 0:1
 * 已证明可用），此处直接复用，只打印寄存器供 bring-up 诊断。若真机读
 * 到全零/全 F，说明 MMIO 窗口或基址不对，先查这一行。
 *
 * 该钩子在 g_ehci.exclsem 创建之后被调用（usb_hc_init），置 g_usb_hc_ready
 * 让 Rust 轮询线程（usb_poller）等 hc_init 完成再开始轮询，避免提前把
 * USBSTS 残留位当作事件提交 bottomhalf。
 */
static volatile uint32_t g_usb_hc_ready = 0;

int sudoos_usb_hc_ready(void)
{
    return (int)g_usb_hc_ready;
}

void usb_hc_low_level_init(void)
{
    volatile const uint32_t *hccr =
        (volatile const uint32_t *)CONFIG_USB_EHCI_HCCR_BASE;
    volatile const uint32_t *hcor =
        (volatile const uint32_t *)CONFIG_USB_EHCI_HCOR_BASE;

    printf("USB-EHCI: HCCR[0..3]=%08x %08x %08x %08x\r\n",
           hccr[0], hccr[1], hccr[2], hccr[3]);
    printf("USB-EHCI: USBCMD=%08x USBSTS=%08x PORTSC=%08x\r\n",
           hcor[0], hcor[2], hcor[0x44 / 4]);

    /* 内存屏障：确保上面寄存器写入先于标志发布（Rust 侧读）。 */
    __asm__ volatile("dbar 0" : : : "memory");
    g_usb_hc_ready = 1;
}

/* ---- 端口速度（覆盖 __WEAK 默认：读取 PORTSC bits[13:10]）----
 *
 * 标准 EHCI 2.0 PORTSC 的 PORTSPD 字段：
 *   0b00 full-speed, 0b01 low-speed, 0b10 high-speed, 0b11 reserved。
 * vendored 头把 bits[13:10] 按 EHCI 1.0 的 OWNER/PIC 解读，2K1000 是
 * 2.0 控制器，直接按 PORTSPD 解码。
 */
uint8_t usbh_get_port_speed(const uint8_t port)
{
    volatile const uint32_t *portsc =
        (volatile const uint32_t *)(CONFIG_USB_EHCI_HCOR_BASE + EHCI_PORTSC_OFFSET(port));
    uint32_t spd = (*portsc >> 10) & 0x3u;

    switch (spd) {
    case 0x1:
        return USB_SPEED_LOW;
    case 0x2:
        return USB_SPEED_HIGH;
    default:
        return USB_SPEED_FULL;
    }
}

/* ---- 轮询等待设备枚举并回填 VID:PID（M2 验收点）----
 *
 * 由 kernel/src/cusb.rs 的 usb_monitor 线程调用；psc 线程异步枚举，这里
 * 轮询 usbh_get_roothub_vid_pid 直到超时。
 */
extern int usbh_get_roothub_vid_pid(uint8_t rhport, uint16_t *vid, uint16_t *pid);
extern unsigned int sudoos_usb_get_tick_ms(void);

int sudoos_usb_wait_device(uint32_t timeout_ms, uint16_t *vid, uint16_t *pid)
{
    uint32_t deadline = sudoos_usb_get_tick_ms() + timeout_ms;

    if (vid == NULL || pid == NULL) {
        return -2;
    }
    do {
        if (usbh_get_roothub_vid_pid(0, vid, pid) == 0) {
            return 0;
        }
        sudoos_usb_sleep_ms(50);
    } while ((int32_t)(sudoos_usb_get_tick_ms() - deadline) < 0);
    return -1;
}

/* ---- CherryUSB 宿主初始化（M2：单次，psc 线程内自调 usb_hc_init）----
 *
 * 只调 `usbh_initialize()`：它创建 psc/hpworkq/lpworkq 三个真实内核线程
 * （usb_osal_sudoos.c 的 M2 线程实现）。psc 线程启动后在临界区内调用
 * `usb_hc_init()`（usbh_core.c），M1 在此处的显式 hc_init 已移除。
 * 返回 0 成功；负值失败。
 */
int sudoos_usb_init(void)
{
    return usbh_initialize();
}
