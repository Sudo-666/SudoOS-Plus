# M7-A：最小用户模式闭环

本交付基于 `main@489e97bf2fb3334d68c9f5a7a2db595ece023a4a`。

## 完成边界

M7-A 实现：

- RISC-V U-mode 与 LoongArch PLV3 同步入口；
- 用户 trap 使用当前内核任务的 kernel stack；
- Linux 通用 64 位 syscall 编号：`write=64`、`exit=93`；
- RISC-V：a7 编号、a0..a5 参数、a0 返回；
- LoongArch：a7(r11) 编号、a0..a5(r4..r9) 参数、a0(r4) 返回；
- 三个临时低地址用户映射：
  - RX code；
  - RW data；
  - RW stack；
- `copy_from_user` / `copy_to_user`：
  - 先做范围和权限检查；
  - 再从物理 backing copy；
  - 不直接解引用用户虚拟地址；
  - 不全局开启 RISC-V SUM；
- 未知 syscall 返回 `-ENOSYS`；
- 用户异常或 page fault 终止当前 M7 session，不把普通用户错误升级为 kernel panic；
- `sys_exit` 经 trap frame 回到原内核调用栈；
- 映射、页表中间页与 backing page 全部回收；
- 双架构 smoke evidence。

## Linux-like 约束

Linux 不把未经检查的用户指针当成普通内核指针。M7-A 同样把
`access_ok` 风格的范围验证和实际 copy 分开。

用户错误只杀死用户执行上下文，内核错误继续 fail-fast。M7-A 还没有
Process，因此使用单一同步 session 表达这个边界。

每个用户线程最终都应拥有独立 kernel stack。M7-A 复用当前 boot task
的 kernel stack，并在整个用户 round trip 中关闭本地中断，保证它不会
被调度器切走。这是显式阶段限制，不是假装已经支持可抢占用户进程。

## 暂不包含

- 独立 per-process page-table root；
- ASID；
- demand paging；
- 用户栈增长；
- Process/Thread；
- ELF；
- fork/exec；
- VFS；
- signal；
- 动态链接；
- 实体机认证。

这些分别属于 M8 以后。

## 预期输出

```text
hello user
minimal user mode test:
  U-mode/PLV3 entry : verified
  user trap stack   : verified
  write/exit ABI    : verified
  checked user copy : verified
  RX/RW W^X pages   : verified
  mapping reclaim   : verified
```
