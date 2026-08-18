#include "usb_list.h"
#include "usb_osal.h"
#include "usb_workq.h"
#include "usb_config.h"

void usb_workqueue_submit(struct usb_workqueue *queue, struct usb_work *work, usb_worker_t worker, void *arg, uint32_t ticks)
{
    uint32_t flags;
    flags = usb_osal_enter_critical_section();

    /* 首次提交的 work 节点可能仍处全零状态（如 g_ehci.work 经 memset 清零）：
     * 对全零节点执行 usb_dlist_remove() 会向 NULL->prev(NULL+8) 写入，page fault
     * address=0x8 access=Write（真机复现）。已初始化的节点按原语义从旧链表摘除。 */
    if (work->list.next == NULL || work->list.prev == NULL) {
        usb_dlist_init(&work->list);
    } else {
        usb_dlist_remove(&work->list);
    }
    work->worker = worker;
    work->arg = arg;

    if (ticks == 0) {
        usb_dlist_insert_after(&queue->work_list, &work->list);
        usb_osal_sem_give(queue->sem);
    }

    usb_osal_leave_critical_section(flags);
}

struct usb_workqueue g_hpworkq = { NULL };
struct usb_workqueue g_lpworkq = { NULL };

static void usbh_hpwork_thread(void *argument)
{
    struct usb_work *work;
    uint32_t flags;
    int ret;
    struct usb_workqueue *queue = (struct usb_workqueue *)argument;
    while (1) {
        ret = usb_osal_sem_take(queue->sem);
        if (ret < 0) {
            continue;
        }
        flags = usb_osal_enter_critical_section();
        if (usb_dlist_isempty(&queue->work_list)) {
            usb_osal_leave_critical_section(flags);
            continue;
        }
        work = usb_dlist_first_entry(&queue->work_list, struct usb_work, list);
        usb_dlist_remove(&work->list);
        usb_osal_leave_critical_section(flags);
        work->worker(work->arg);
    }
}

static void usbh_lpwork_thread(void *argument)
{
    struct usb_work *work;
    uint32_t flags;
    int ret;
    struct usb_workqueue *queue = (struct usb_workqueue *)argument;
    while (1) {
        ret = usb_osal_sem_take(queue->sem);
        if (ret < 0) {
            continue;
        }
        flags = usb_osal_enter_critical_section();
        if (usb_dlist_isempty(&queue->work_list)) {
            usb_osal_leave_critical_section(flags);
            continue;
        }
        work = usb_dlist_first_entry(&queue->work_list, struct usb_work, list);
        usb_dlist_remove(&work->list);
        usb_osal_leave_critical_section(flags);
        work->worker(work->arg);
    }
}

int usbh_workq_initialize(void)
{
    /* g_hpworkq/g_lpworkq 以 `{ NULL }` 静态初始化，链表头 work_list /
     * delay_work_list 全零。usb_dlist_insert_after 对空链表写 l->next->prev
     * （NULL+8），usb_dlist_isempty 判 `next == head` 对全零头恒假 → page
     * fault address=0x8 access=Write（真机复现）。必须先自环。 */
    usb_dlist_init(&g_hpworkq.work_list);
    usb_dlist_init(&g_hpworkq.delay_work_list);
    usb_dlist_init(&g_lpworkq.work_list);
    usb_dlist_init(&g_lpworkq.delay_work_list);

    g_hpworkq.sem = usb_osal_sem_create(0);
    if (g_hpworkq.sem == NULL) {
        return -1;
    }
    g_hpworkq.thread = usb_osal_thread_create("usbh_hpworkq", CONFIG_USBHOST_HPWORKQ_STACKSIZE, CONFIG_USBHOST_HPWORKQ_PRIO, usbh_hpwork_thread, &g_hpworkq);
    if (g_hpworkq.thread == NULL) {
        return -1;
    }

    g_lpworkq.sem = usb_osal_sem_create(0);
    if (g_lpworkq.sem == NULL) {
        return -1;
    }

    g_lpworkq.thread = usb_osal_thread_create("usbh_lpworkq", CONFIG_USBHOST_LPWORKQ_STACKSIZE, CONFIG_USBHOST_LPWORKQ_PRIO, usbh_lpwork_thread, &g_lpworkq);
    if (g_lpworkq.thread == NULL) {
        return -1;
    }
    return 0;
}
