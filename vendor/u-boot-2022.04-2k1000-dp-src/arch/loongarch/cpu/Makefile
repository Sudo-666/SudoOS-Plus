# SPDX-License-Identifier: GPL-2.0+

extra-y	= start.o

obj-y += lowlevel_init.o
obj-y += cpu.o
obj-y += time.o
obj-y += interrupts.o

ifdef CONFIG_SPL_BUILD
obj-y	+= spl.o
endif