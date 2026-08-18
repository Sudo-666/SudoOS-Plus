/*
 * kernel/csrc/usb/usb_osal_sudoos.c
 *
 * CherryUSB OSAL 到 SudoOS 的映射。
 *
 * M1 提供：内存 / 临界区 / 时钟延时（真实实现，经 kernel/src/cusb.rs
 *          导出的 Rust 原语）；线程 / 信号量（桩——host 栈线程尚未接入
 *          SudoOS 调度器）。
 * M2 起：  线程 / 信号量改接 SudoOS 内核线程 + WaitQueue，驱动枚举。
 *
 * 临界区直接用 LoongArch CRMD.IE 位保存/恢复；M2 接调度器后改用内核
 * irq 机制（arch::interrupt）保持中断状态跟踪一致。
 */
#include <stddef.h>
#include <stdint.h>
#include "usb_osal.h"

/* 由 kernel/src/cusb.rs 导出的 Rust 原语。 */
extern void *sudoos_usb_alloc(unsigned long size);
extern void sudoos_usb_free(void *ptr);
extern unsigned int sudoos_usb_get_tick_ms(void);
extern void sudoos_usb_msleep(unsigned int ms);

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

/* ---- 时钟 / 延时（委托 Rust，见 kernel/src/cusb.rs）---- */
void usb_osal_msleep(uint32_t delay)
{
    sudoos_usb_msleep(delay);
}

uint32_t usb_osal_get_tick(void)
{
    return sudoos_usb_get_tick_ms();
}

/* ---- 线程（M1 桩；M2 接 SudoOS 内核线程）---- */
usb_osal_thread_t usb_osal_thread_create(const char *name, uint32_t stack_size,
                                         uint32_t prio, usb_thread_entry_t entry,
                                         void *args)
{
    (void)name;
    (void)stack_size;
    (void)prio;
    (void)entry;
    (void)args;
    /* 返回非 NULL，让 usbh_initialize/usb_workq_initialize 判定成功；
       线程体由 M2 真正调度。 */
    return (usb_osal_thread_t)0x1;
}

void usb_osal_thread_delete(usb_osal_thread_t thread)
{
    (void)thread;
}

void usb_osal_thread_suspend(usb_osal_thread_t thread)
{
    (void)thread;
}

void usb_osal_thread_resume(usb_osal_thread_t thread)
{
    (void)thread;
}

/* ---- 信号量（M1：计数 + 有界自旋；M2 改 WaitQueue 阻塞）---- */
typedef struct {
    int count;
} sudoos_usb_sem;

usb_osal_sem_t usb_osal_sem_create(uint32_t initial_count)
{
    sudoos_usb_sem *sem = sudoos_usb_alloc(sizeof(*sem));
    if (sem == NULL) {
        return NULL;
    }
    sem->count = (int)initial_count;
    return (usb_osal_sem_t)sem;
}

void usb_osal_sem_delete(usb_osal_sem_t sem)
{
    sudoos_usb_free(sem);
}

int usb_osal_sem_take(usb_osal_sem_t sem)
{
    sudoos_usb_sem *s = (sudoos_usb_sem *)sem;
    /* 有界自旋兜底，避免 M1 桩阶段意外死等；M2 改真实阻塞。 */
    unsigned int deadline = sudoos_usb_get_tick_ms() + 30000u;
    while (s->count <= 0) {
        if ((int)(sudoos_usb_get_tick_ms() - deadline) >= 0) {
            return -1;
        }
    }
    s->count--;
    return 0;
}

int usb_osal_sem_give(usb_osal_sem_t sem)
{
    ((sudoos_usb_sem *)sem)->count++;
    return 0;
}

/* ---- 互斥锁（M1：计数为 1 的信号量）---- */
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
