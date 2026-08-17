# 双板竞赛存储验收（CodePlan C1–C10）

没有正式竞赛镜像时，用运行时生成的小型 ext4 fixture 在真机/仿真上验证公共
竞赛存储链：

```
BlockDevice → ext4 → VFS 挂载 → 脚本发现/文件读取 → OSCOMP
```

存储设备按平台选择：

| 平台 | 设备 | 说明 |
|------|------|------|
| QEMU VirtIO | `vda` | `-drive` + virtio-blk |
| VisionFive 2 | `mmcblk1` | JH7110 DW-MMC TF 槽 |
| LS2K1000 | `ram0` | U-Boot 加载进预留物理内存 |
| LS2K1000 原生 U 盘 | `sda` | 后续（EHCI+USB 未实施） |

## fixture

```bash
# 32 MiB raw ext4，含 /SUDOOS_CONTEST_FIXTURE、/arch、cagent_testcode.sh 等
scripts/make-contest-fixture.sh --arch riscv64    --output build/fixtures/contest-rv.ext4
scripts/make-contest-fixture.sh --arch loongarch64 --output build/fixtures/contest-la.ext4
python3 scripts/check-contest-fixture.py --arch riscv64 --image build/fixtures/contest-rv.ext4
```

## QEMU（riscv64，可全自动）

```bash
make smoke-contest-fixture-riscv64
# 要求串口出现 CONTEST_FIXTURE: arch=riscv64 + FIXTURE_OSCOMP_PASS
```

loongarch64 的 QEMU 直启还差 FDT 回退（parked 分支），fixture 目标会卡在
启动；loongarch 验证走真机。

## VisionFive 2（TF 卡）

```bash
# 构建 FIT（含 conf-contest-fixture-single / conf-contest-fixture-smp）
VISIONFIVE2_DTB=/path/to/jh7110-...dtb make visionfive2-tftp-bundle

# TFTP 到板上（U-Boot）：
#   setenv bootargs; setenv fdt_high
#   tftpboot 0x60000000 sudoos/vf2/sudoos-visionfive2.itb
#   bootm 0x60000000#conf-contest-fixture-single   (或 #conf-contest-fixture-smp)
```

TF 卡放 `contest-rv.ext4`（32 MiB raw ext4）。bootargs 由 FIT DTB 携带：
`sudoos.oscomp=preliminary sudoos.contest.dev=mmcblk1 sudoos.contest.fixture=1
sudoos.maxcpus=1|4`（无 `rdinit=/init`）。

## LS2K1000（U-Boot 内存盘）

```bash
make ls2k1000-contest-fixture-bundle      # kernel uImage + DTB + contest-la.ext4 + boot.txt
make ls2k1000-contest-fixture-configs     # single/smp DTB 变体

# U-Boot（cached-VA）：
#   fatload usb 0:1 0x9000000002000000 kernel-ls2k1000.uImage
#   fatload usb 0:1 0x900000000a000000 ls2k1000-contest-fixture-single.dtb
#   fatload usb 0:1 0x90000000e0000000 contest-la.ext4
#   bootm 0x9000000002000000 - 0x900000000a000000
```

DTB 在 `/reserved-memory` 声明 `contest-disk@e0000000`
（`compatible = "sudoos,boot-ramdisk"`），内核据此注册 `/dev/ram0`。

## 日志验收

结果协议（K2.1）：fixture 模式要求 `CONTEST_FIXTURE: paths-missing=0` +
`FIXTURE_OSCOMP_PASS`；正式镜像模式要求每个 runner 都打印
`CONTEST_RESULT mode=<mode> pass`（仅脚本退出码 0 才打印 pass）。

```bash
# fixture（自动生成的 ext4 探测镜像）
python3 scripts/check-board-contest-log.py --board visionfive2 --image-type fixture --fixture vf2-contest.log
python3 scripts/check-board-contest-log.py --board ls2k1000  --image-type fixture --fixture ls2k-contest.log

# final（正式评测镜像）
python3 scripts/check-board-contest-log.py --board visionfive2 --image-type final --fixture vf2-final.log
python3 scripts/check-board-contest-log.py --board ls2k1000  --image-type final --fixture ls2k-final.log
```

公共要求：`CONTEST00..03`（按序）、`SMOKE_TEST: PASS`；fixture 额外要求
`paths-missing=0` + `FIXTURE_OSCOMP_PASS`；final 额外要求
`CONTEST_RESULT mode=final-cagent pass`、`CONTEST_RESULT mode=final-buildstorm pass`
与 `final-image-contract` 全路径 present。

VF2 额外：`VF2-TF00..03`；LS2K 额外：`LS2K-RAMDISK00/01` +
`registered=/dev/ram0`。

拒绝：`FIXTURE_OSCOMP_FAIL`、任何 `CONTEST_RESULT ... fail|timeout`、`panic`、
`timeout`、`CRC error`、`out of range`、`filesystem corrupt`、`unhandled trap`、
`OOM`。
