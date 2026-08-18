/*
 * kernel/csrc/usb/usb_osal_sudoos.c
 *
 * CherryUSB OSAL 到 SudoOS 的映射。
 *
 * M1：内存 / 临界区 / 时钟延时（真实实现）；线程 / 信号量（桩）。
 * M2：线程接 SudoOS 内核线程（KernelThreadEntry = fn()，idx 经 Rust
 *     trampoline 烘焙）；信号量/互斥锁改为 Rust WaitQueue 阻塞实现
 *     （控制块在缓存堆，禁 ll/sc 落在 uncached 窗口）。
 *
 * 分配策略：
 * - usb_osal_malloc/free       → `.nocache_ram` uncached DMA 池
 *   （EHCI 描述符与数据缓冲必须控制器可见且无缓存陈旧问题，见 ADR-001）；
 * - 信号量/互斥锁控制块        → 普通缓存内核堆（sudoos_usb_alloc_ctrl），
 *   与 DMA 池物理隔离，避免 cached/uncached 页别名污染。
 *
 * 传输完成驱动：M2 用 `usb_ehci_interrupt()` 轮询线程（2K1000 无外设
 * 中断基础设施），见 kernel/src/cusb.rs 的 usb_poller。
 */
#include <stddef.h>
#include <stdint.h>
#include "usb_osal.h"

/* 由 kernel/src/cusb.rs 导出的 Rust 原语。 */
extern void *sudoos_usb_alloc(unsigned long size);
extern void sudoos_usb_free(void *ptr);
extern void *sudoos_usb_alloc_ctrl(unsigned long size);
extern void sudoos_usb_free_ctrl(void *ptr);
extern unsigned int sudoos_usb_get_tick_ms(void);
extern void sudoos_usb_msleep(unsigned int ms);     /* busy-wait（boot 上下文） */
extern void sudoos_usb_sleep_ms(unsigned int ms);   /* task sleep（C 线程内） */
extern void *sudoos_usb_sem_create(int initial);
extern void sudoos_usb_sem_delete(void *sem);
extern int sudoos_usb_sem_take(void *sem, unsigned int timeout_ms);
extern int sudoos_usb_sem_give(void *sem);
extern int sudoos_usb_thread_spawn(unsigned int idx);

/* 本文件定义：是否已进入 C 线程体（决定 msleep 走任务睡眠）。 */
int sudoos_usb_in_thread(void);

/* ---- 内存 ---- */
void *usb_osal_malloc(uint32_t size)
{
    return sudoos_usb_alloc(size);
}

void usb_osal_free(void *ptr)
{
    sudoos_usb_free(ptr);
}

/* ---- 临界区（LoongArch CRMD.IE）---- */
static inline uint32_t sudoos_usb_read_crmd(void)
{
    uint32_t crmd;
    __asm__ volatile("csrrd %0, 0x0" : "=r"(crmd));
    return crmd;
}

uint32_t usb_osal_enter_critical_section(void)
{
    uint32_t crmd = sudoos_usb_read_crmd();
    __asm__ volatile("csrwr %0, 0x0" : : "r"(crmd & ~0x1u));
    return crmd & 0x1u;
}

void usb_osal_leave_critical_section(uint32_t flag)
{
    if (flag != 0) {
        __asm__ volatile("csrwr %0, 0x0" : : "r"(sudoos_usb_read_crmd() | 0x1u));
    }
}

/* ---- 时钟 / 延时（M2：C 线程内用真实任务睡眠，boot 上下文忙等）---- */
void usb_osal_msleep(uint32_t delay)
{
    if (sudoos_usb_in_thread()) {
        sudoos_usb_sleep_ms(delay);
    } else {
        sudoos_usb_msleep(delay);
    }
}

uint32_t usb_osal_get_tick(void)
{
    return sudoos_usb_get_tick_ms();
}

/* ---- 线程（M2：SudoOS 内核线程）----
 *
 * KernelThreadEntry = fn()（无参），而 CherryUSB 线程是 (entry, args) 对。
 * Rust 侧用宏烘焙出 USB_THREAD_SLOTS 个无参 trampoline，各 trampoline 把
 * 槽位 idx 传给本文件的 sudoos_usb_thread_entry，后者按注册表分派。
 */
#define SUDOOS_USB_MAX_THREADS 8

typedef struct {
    usb_thread_entry_t entry;
    void *args;
} sudoos_usb_thread_ctx;

static sudoos_usb_thread_ctx g_thread_ctx[SUDOOS_USB_MAX_THREADS];
static uint32_t g_thread_next = 0;

/* 是否已进入 C 线程体：决定 msleep 走任务睡眠还是忙等。 */
static volatile uint32_t g_usb_in_thread = 0;

int sudoos_usb_in_thread(void)
{
    return (int)g_usb_in_thread;
}

/* Rust trampoline 调用：在新建内核线程的栈上运行 CherryUSB 线程体。 */
void sudoos_usb_thread_entry(uint32_t idx)
{
    g_usb_in_thread = 1;
    if (idx < SUDOOS_USB_MAX_THREADS && g_thread_ctx[idx].entry != NULL) {
        g_thread_ctx[idx].entry(g_thread_ctx[idx].args);
    }
    /* CherryUSB 线程体永不返回；万一返回，挂起兜底避免销毁线程栈。 */
    for (;;) {
        sudoos_usb_sleep_ms(1000);
    }
}

usb_osal_thread_t usb_osal_thread_create(const char *name, uint32_t stack_size,
                                         uint32_t prio, usb_thread_entry_t entry,
                                         void *args)
{
    uint32_t idx;

    (void)name;
    (void)stack_size;
    (void)prio;

    if (entry == NULL || g_thread_next >= SUDOOS_USB_MAX_THREADS) {
        return NULL;
    }
    idx = g_thread_next++;
    g_thread_ctx[idx].entry = entry;
    g_thread_ctx[idx].args = args;
    if (sudoos_usb_thread_spawn(idx) != 0) {
        g_thread_next--;
        g_thread_ctx[idx].entry = NULL;
        return NULL;
    }
    return (usb_osal_thread_t)(uintptr_t)(idx + 1);
}

void usb_osal_thread_delete(usb_osal_thread_t thread)
{
    (void)thread;
}

/*
 * M2 保持 no-op：psc 线程在枚举期 suspend lpworkq 是为防止异步传输并发；
 * 而枚举只跑同步 control transfer（hpworkq 驱动），lpworkq 此时无异步工作。
 * M3 若引入中断式传输再补真实挂起语义。
 */
void usb_osal_thread_suspend(usb_osal_thread_t thread)
{
    (void)thread;
}

void usb_osal_thread_resume(usb_osal_thread_t thread)
{
    (void)thread;
}

/* ---- 信号量（M2：Rust WaitQueue 阻塞实现，控制块在缓存堆）---- */
usb_osal_sem_t usb_osal_sem_create(uint32_t initial_count)
{
    return (usb_osal_sem_t)sudoos_usb_sem_create((int)initial_count);
}

void usb_osal_sem_delete(usb_osal_sem_t sem)
{
    sudoos_usb_sem_delete(sem);
}

int usb_osal_sem_take(usb_osal_sem_t sem)
{
    /* 有界等待：挂起的控制器最终解阻塞，便于真机诊断。 */
    return sudoos_usb_sem_take(sem, 60000u);
}

int usb_osal_sem_give(usb_osal_sem_t sem)
{
    return sudoos_usb_sem_give(sem);
}

/* ---- 互斥锁（计数为 1 的信号量，同 M1）---- */
usb_osal_mutex_t usb_osal_mutex_create(void)
{
    return usb_osal_sem_create(1);
}

void usb_osal_mutex_delete(usb_osal_mutex_t mutex)
{
    usb_osal_sem_delete(mutex);
}

int usb_osal_mutex_take(usb_osal_mutex_t mutex)
{
    return usb_osal_sem_take(mutex);
}

int usb_osal_mutex_give(usb_osal_mutex_t mutex)
{
    return usb_osal_sem_give(mutex);
}
