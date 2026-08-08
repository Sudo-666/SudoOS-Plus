#
# (C) Copyright 2019
# <tang-haifeng@foxmail.com> <pengren.mcu@qq.com>
#
# SPDX-License-Identifier:      GPL-2.0+
#

obj-$(CONFIG_MACH_LOONGSON)			+= platform.o

ifndef CONFIG_SPL_BUILD
obj-$(CONFIG_MACH_LOONGSON)			+= loongson_update.o
obj-$(CONFIG_LOONGSON_RECOVER)		+= recover.o
obj-$(CONFIG_LOONGSON_BOOT_FIXUP)	+= bootparam.o loongson_efi_systab.o
obj-$(CONFIG_LOONGSON_COMPAT)		+= multi_boards_compat.o
obj-y += loongson_boot.o
obj-y += loongson_env_trigger.o
obj-y += loongson_init_env.o

ifeq ($(CONFIG_MENU), y)
obj-$(CONFIG_MACH_LOONGSON) += loongson_update_menu.o
endif
endif
