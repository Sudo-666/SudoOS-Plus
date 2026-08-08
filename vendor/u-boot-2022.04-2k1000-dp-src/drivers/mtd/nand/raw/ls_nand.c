// SPDX-License-Identifier: GPL-2.0
#include <dm.h>
#include <common.h>
#include <malloc.h>
#include <nand.h>
#include <linux/mtd/mtd.h>
#include <linux/mtd/nand.h>
#include <linux/mtd/rawnand.h>
#include <linux/mtd/partitions.h>
#include <linux/mtd/nand_bch.h>
#include <linux/delay.h>
#include <asm/io.h>
#include <dma.h>

#define LS_NAND_MAX_CHIPS	4
#define ALIGN_DMA(x)	((x + 3)/4)
#define REG(reg)	(info->mmio_base + reg)

#define MAX_BUFF_SIZE		0x10000
#define STATUS_TIME_LOOP_R	15000
#define STATUS_TIME_LOOP_WS	5000
#define STATUS_TIME_LOOP_WM	15000
#define STATUS_TIME_LOOP_E	500000

// #define STATUS_TIME_LOOP_R	30000

/* Register offset */
#define NAND_CMD_REG		0x00
#define NAND_ADDRC_REG		0x04
#define NAND_ADDRR_REG		0x08
#define NAND_TIM_REG		0x0c
#define NAND_IDL_REG		0x10
#define NAND_IDH_REG		0x14
#define NAND_STA_REG		0x14
#define NAND_PARAM_REG		0x18
#define NAND_OP_NUM_REG		0x1c
#define NAND_CS_RDY_REG		0x20
#define NAND_DMA_ADDR_REG	0x40

/* NAND_CMD_REG */
#define CMD_VALID		(1 << 0)	/* command valid */
#define CMD_RD_OP		(1 << 1)	/* read operation */
#define CMD_WR_OP		(1 << 2)	/* write operation */
#define CMD_ER_OP		(1 << 3)	/* erase operation */
#define CMD_BER_OP		(1 << 4)	/* blocks erase operation */
#define CMD_RD_ID		(1 << 5)	/* read id */
#define CMD_RESET		(1 << 6)	/* reset */
#define CMD_RD_STATUS		(1 << 7)	/* read status */
#define CMD_MAIN		(1 << 8)	/* operation in main region */
#define CMD_SPARE		(1 << 9)	/* operation in spare region */
#define CMD_DONE		(1 << 10)	/* operation done */
#define CMD_RS_RD		(1 << 11)	/* ecc is enable when reading */
#define CMD_RS_WR		(1 << 12)	/* ecc is enable when writing */
#define CMD_INT_EN		(1 << 13)	/* interrupt enable */
#define CMD_WAIT_RS		(1 << 14)	/* waiting ecc read done */
#define CMD_ECC_DMA_REQ		(1 << 30)	/* dma request in ecc mode */
#define CMD_DMA_REQ		(1 << 31)	/* dma request in normal mode */
#define CMD_RDY_SHIF		16		/* four bits for chip ready */
#define CMD_CE_SHIF		20		/* four bits for chip enable */

/* NAND_TIM_REG */
#define HOLD_CYCLE		0x04
#define WAIT_CYCLE		0x12

/* NAND_PARAM_REG */
#define CHIP_CAP_SHIFT		8
#define ID_NUM_SHIFT		12
#define OP_SCOPE_SHIFT		16

#define nand_writel(info, off, val)	\
	__raw_writel((val), (info)->mmio_base + (off))

#define nand_readl(info, off)		\
	__raw_readl((info)->mmio_base + (off))

struct ls_nand_info {
	struct nand_chip	nand_chip;
	struct mtd_info		*mtd;
	spinlock_t		nand_lock;

	/* MTD data control */
	unsigned int		buf_start;
	unsigned int		buf_count;

	void __iomem		*mmio_base;
	unsigned int		cmd;

	struct dma			dma_chan;
	dma_addr_t			apb_data_addr;

	unsigned char		*data_buff;	/* dma data buffer */
	size_t			data_buff_size;
	size_t			data_size;	/* data size in FIFO */
	unsigned int		seqin_column;
	unsigned int		seqin_page_addr;
	int			cs, cs0;
	u32			csrdy;
	int			chip_cap;
};

// static unsigned int bch = 4;
static unsigned int bch = 0;

static void wait_nand_done(struct ls_nand_info *info, int timeout);
static int ls_nand_init_buff(struct ls_nand_info *info)
{
	info->data_buff = memalign(ARCH_DMA_MINALIGN, MAX_BUFF_SIZE);
	if (info->data_buff == NULL) {
		pr_err("failed to allocate dma buffer\n");
		return -ENOMEM;
	}
	info->data_buff_size = MAX_BUFF_SIZE;
	return 0;
}

static int ls_nand_ecc_calculate(struct mtd_info *mtd,
				const uint8_t *dat, uint8_t *ecc_code)
{
	return 0;
}

static int ls_nand_ecc_correct(struct mtd_info *mtd,
				uint8_t *dat, uint8_t *read_ecc,
				uint8_t *calc_ecc)
{
	/*
	 * Any error include ERR_SEND_CMD, ERR_DBERR, ERR_BUSERR, we
	 * consider it as a ecc error which will tell the caller the
	 * read fail We have distinguish all the errors, but the
	 * nand_read_ecc only check this function return value
	 */
	return 0;
}

static void ls_nand_ecc_hwctl(struct mtd_info *mtd, int mode)
{

}

static int ls_nand_get_ready(struct mtd_info *mtd)
{
	struct ls_nand_info *info = mtd->priv;
	unsigned char status;

	writel(CMD_RD_STATUS, REG(NAND_CMD_REG));
	writel(CMD_RD_STATUS | CMD_VALID, REG(NAND_CMD_REG));
	wait_nand_done(info, STATUS_TIME_LOOP_R);
	status = readl(REG(NAND_IDH_REG))>>16;
	return status;
}

static int ls_nand_waitfunc(struct mtd_info *mtd, struct nand_chip *this)
{
	struct ls_nand_info *info = mtd->priv;
	unsigned char status;

	status = readl(REG(NAND_IDH_REG))>>16;
	return status;
}

static void ls_nand_select_chip(struct mtd_info *mtd, int chip)
{
	struct ls_nand_info *info = mtd->priv;

	if (chip == -1 || chip == 0)
		info->cs = info->cs0;
	else
		info->cs = (info->cs0 + chip) % LS_NAND_MAX_CHIPS;
}

static int ls_nand_dev_ready(struct mtd_info *mtd)
{
	struct ls_nand_info *info = mtd->priv;

	return	!!(readl(REG(NAND_CMD_REG)) & (1<<(info->cs+16)));
}

static const char cap2cs[16] = {[0] = 16, [1]  = 17, [2]  = 18, [3] = 19,
				[4] = 19, [5]  = 19, [6]  = 20, [7] = 21,
				[9] = 14, [10] = 15, [11] = 16, [12] = 17,
				[13] = 18};

static void nand_setup(struct ls_nand_info *info,
		int cmd, int addr_c, int addr_r, int param, int op_num)
{
	unsigned int addr_cs = info->cs*(1UL<<cap2cs[(param>>CHIP_CAP_SHIFT)&0xf]);

	writel(param, REG(NAND_PARAM_REG));
	writel(op_num, REG(NAND_OP_NUM_REG));
	writel(addr_c, REG(NAND_ADDRC_REG));
	writel(addr_r|addr_cs, REG(NAND_ADDRR_REG));
	writel(0, REG(NAND_CMD_REG));
	writel(cmd & (~CMD_VALID), REG(NAND_CMD_REG));
	writel(cmd | CMD_VALID, REG(NAND_CMD_REG));
}

static void wait_nand_done(struct ls_nand_info *info, int timeout_us)
{
	int t, status_times = timeout_us / 10;

	do {
		t = readl(REG(NAND_CMD_REG)) & CMD_DONE;
		if (!(status_times--)) {
			writel(0x0, REG(NAND_CMD_REG));
			writel(CMD_RESET, REG(NAND_CMD_REG));
			writel(CMD_RESET | CMD_VALID, REG(NAND_CMD_REG));
			break;
		}
		udelay(10);
	} while (t == 0);

	if (status_times <= 0) {
		debug("Wait nand timeout\n");
	}

	writel(0x0, REG(NAND_CMD_REG));
}

static void dma_xfer(struct ls_nand_info *info, int is_send, int len)
{
	dma_enable(&info->dma_chan);
	if (is_send) {
		if (dma_send(&info->dma_chan, info->data_buff, 
					len, (void *)info->apb_data_addr)) {
			printf("Nand DMA send failed!\n");
			goto failed;
		}
		
	} else {
		if (dma_prepare_rcv_buf(&info->dma_chan, info->data_buff, len)) {
			printf("Nand DMA recv prepare failed!\n");
			goto failed;
	}

		if (dma_receive(&info->dma_chan, NULL, (void *)info->apb_data_addr)) {
			printf("Nand DMA recv failed!\n");
			goto failed;
		}
	}

	wait_nand_done(info, STATUS_TIME_LOOP_R);

failed:
	dma_disable(&info->dma_chan);
}

static int get_chip_capa_num(uint64_t  chipsize, int pagesize)
{
	int size_mb = chipsize >> 20;

	if (pagesize == 4096)
		return 4;
	else if (pagesize == 2048)
		switch (size_mb) {
		case (1 << 7):		/* 1Gb */
			return 0;
		case (1 << 8):		/* 2Gb */
			return 1;
		case (1 << 9):		/* 4Gb */
			return 2;
		case (1 << 10):		/* 8Gb */
		default:
			return 3;
		}
	else if (pagesize == 8192)

		switch (size_mb) {
		case (1 << 12):		/* 32Gb */
			return 5;
		case (1 << 13):		/* 64Gb */
			return 6;
		case (1 << 14):		/* 128Gb */
		default:
			return 7;
		}
	else if (pagesize == 512)

		switch (size_mb) {
		case (1 << 3):		/* 64Mb */
			return 9;
		case (1 << 4):		/* 128Mb */
			return 0xa;
		case (1 << 5):		/* 256Mb */
			return 0xb;
		case (1 << 6):		/* 512Mb */
		default:
			return 0xc;
		}
	else
		return 0;
}

static void ls_read_id(struct ls_nand_info *info)
{
	unsigned int id_l, id_h;
	unsigned char *data = (unsigned char *)(info->data_buff);
	unsigned int addr_cs, chip_cap;
	struct mtd_info *mtd = nand_to_mtd(&info->nand_chip);

	if (mtd->writesize) {
		chip_cap = get_chip_capa_num(info->nand_chip.chipsize, mtd->writesize);
		addr_cs = info->cs*(1UL<<cap2cs[chip_cap]);
		info->chip_cap  = chip_cap;
	} else  {
		addr_cs = info->cs*(1UL<<cap2cs[info->chip_cap]);
	}

	writel((2048 << OP_SCOPE_SHIFT) | (6 << ID_NUM_SHIFT) | (info->chip_cap << CHIP_CAP_SHIFT), REG(NAND_PARAM_REG));
	writel(addr_cs, REG(NAND_ADDRR_REG));
	writel(0, REG(NAND_CMD_REG));
	writel(CMD_RD_ID, REG(NAND_CMD_REG));
	writel((CMD_RD_ID | CMD_VALID), REG(NAND_CMD_REG));
	wait_nand_done(info, 10000);

	id_l = readl(REG(NAND_IDL_REG));
	id_h = readl(REG(NAND_IDH_REG));
	pr_debug("id_l: %08x, id_h:%08x\n", id_l, id_h);
	data[0] = ((id_h >> 8) & 0xff);
	data[1] = (id_h & 0xff);
	data[2] = (id_l >> 24) & 0xff;
	data[3] = (id_l >> 16) & 0xff;
	data[4] = (id_l >> 8) & 0xff;
	data[5] = id_l & 0xff;
	data[6] = 0;
	data[7] = 0;
}

static void ls_nand_cmdfunc(struct mtd_info *mtd, unsigned int command,
			      int column, int page_addr)
{
	struct ls_nand_info *info = mtd->priv;
	int chip_cap, oobsize, pagesize;
	int cmd, addrc, addrr, op_num, param;

	info->cmd = command;
	oobsize = mtd->oobsize;
	pagesize = mtd->writesize;
	chip_cap = get_chip_capa_num(info->nand_chip.chipsize, pagesize);

	switch (command) {
	case NAND_CMD_READOOB:
		info->buf_count = oobsize;
		if (info->buf_count <= 0)
			break;
		info->buf_start = column;
		addrc = pagesize;
		addrr = page_addr;
		param = (oobsize << OP_SCOPE_SHIFT)
			| (chip_cap << CHIP_CAP_SHIFT);
		op_num = oobsize;
		cmd = CMD_SPARE | CMD_RD_OP;
		nand_setup(info, cmd, addrc, addrr, param, op_num);
		dma_xfer(info, 0, op_num);
		break;
	case NAND_CMD_READ0:
		addrc = 0;
		addrr = page_addr;
		op_num = oobsize + pagesize;
		param = (op_num << OP_SCOPE_SHIFT) | (chip_cap << CHIP_CAP_SHIFT);
		cmd = CMD_SPARE | CMD_RD_OP | CMD_MAIN;
		info->buf_count = op_num;
		info->buf_start = column;
		nand_setup(info, cmd, addrc, addrr, param, op_num);
		dma_xfer(info, 0, op_num);
		break;
	case NAND_CMD_SEQIN:
		info->buf_count = oobsize + pagesize - column;
		info->buf_start = 0;
		info->seqin_column = column;
		info->seqin_page_addr = page_addr;
		break;
	case NAND_CMD_PAGEPROG:
		addrc = info->seqin_column;
		addrr = info->seqin_page_addr;
		op_num = info->buf_start;
		param = ((pagesize + oobsize) << OP_SCOPE_SHIFT)
			| (chip_cap << CHIP_CAP_SHIFT);
		cmd = CMD_SPARE | CMD_WR_OP;
		if (addrc < pagesize)
			cmd |= CMD_MAIN;
		nand_setup(info, cmd, addrc, addrr, param, op_num);
		dma_xfer(info, 1, op_num);
		break;
	case NAND_CMD_RESET:
		nand_setup(info, CMD_RESET, 0, 0, 0, 0);
		wait_nand_done(info, STATUS_TIME_LOOP_R);
		break;
	case NAND_CMD_ERASE1:
		addrc = 0;
		addrr = page_addr;
		op_num = 1;
		param = ((pagesize + oobsize) << OP_SCOPE_SHIFT)
			| (chip_cap << CHIP_CAP_SHIFT);
		cmd = CMD_ER_OP;
		nand_setup(info, cmd, addrc, addrr, param, op_num);
		wait_nand_done(info, STATUS_TIME_LOOP_E);
		break;
	case NAND_CMD_STATUS:
		info->buf_count = 0x1;
		info->buf_start = 0x0;
		*(unsigned char *)info->data_buff =
			ls_nand_get_ready(mtd);
		break;
	case NAND_CMD_READID:
		info->buf_count = 0x6;
		info->buf_start = 0;
		ls_read_id(info);
		break;
	case NAND_CMD_ERASE2:
	case NAND_CMD_READ1:
		break;
	case NAND_CMD_RNDOUT:
		info->buf_start = column;
		break;
	default:
		printk(KERN_ERR "non-supported command.\n");
		break;
	}
}

static u16 ls_nand_read_word(struct mtd_info *mtd)
{
	struct ls_nand_info *info = mtd->priv;
	u16 retval = 0xFFFF;

	if (!(info->buf_start & 0x1) && info->buf_start < info->buf_count)
		retval = *(u16 *) (info->data_buff + info->buf_start);
	info->buf_start += 2;

	return retval;
}

static uint8_t ls_nand_read_byte(struct mtd_info *mtd)
{
	struct ls_nand_info *info = mtd->priv;
	char retval = 0xFF;

	if (info->buf_start < info->buf_count)
		retval = info->data_buff[(info->buf_start)++];

	return retval;
}

static void ls_nand_read_buf(struct mtd_info *mtd, uint8_t *buf, int len)
{
	struct ls_nand_info *info = mtd->priv;
	int real_len = min_t(size_t, len, info->buf_count - info->buf_start);

	memcpy(buf, info->data_buff + info->buf_start, real_len);

	info->buf_start += real_len;
}

static void ls_nand_write_buf(struct mtd_info *mtd, const uint8_t *buf,
				int len)
{
	struct ls_nand_info *info = mtd->priv;
	int real_len = min_t(size_t, len, info->buf_count - info->buf_start);

	memcpy(info->data_buff + info->buf_start, buf, real_len);
	info->buf_start += real_len;
}

static void ls_nand_init_mtd(struct mtd_info *mtd,
			       struct ls_nand_info *info)
{
	struct nand_chip *this = &info->nand_chip;

	this->options		= NAND_CACHEPRG;
	this->waitfunc		= ls_nand_waitfunc;
	this->select_chip	= ls_nand_select_chip;
	this->dev_ready		= ls_nand_dev_ready;
	this->cmdfunc		= ls_nand_cmdfunc;
	this->read_word		= ls_nand_read_word;
	this->read_byte		= ls_nand_read_byte;
	this->read_buf		= ls_nand_read_buf;
	this->write_buf		= ls_nand_write_buf;
#if defined(CONFIG_NAND_ECC_BCH)
	this->ecc.mode		= NAND_ECC_SOFT_BCH;
#else
	this->ecc.mode		= NAND_ECC_SOFT;
	this->ecc.algo		= NAND_ECC_BCH;
#endif
	this->ecc.hwctl		= ls_nand_ecc_hwctl;
	this->ecc.calculate	= ls_nand_ecc_calculate;
	this->ecc.correct	= ls_nand_ecc_correct;
	this->ecc.size		= 256;
	this->ecc.bytes		= 3;
	mtd->owner = THIS_MODULE;
}

static void ls_nand_init_info(struct ls_nand_info *info)
{
	info->buf_start = 0;
	info->buf_count = 0;
	info->seqin_column = 0;
	info->seqin_page_addr = 0;
	spin_lock_init(&info->nand_lock);
}

static void ls1x_nand_init_hw(struct ls_nand_info *info)
{
	nand_writel(info, NAND_CMD_REG, 0x00);
	nand_writel(info, NAND_ADDRC_REG, 0x00);
	nand_writel(info, NAND_ADDRR_REG, 0x00);
	nand_writel(info, NAND_TIM_REG, (HOLD_CYCLE << 8) | WAIT_CYCLE);
	nand_writel(info, NAND_PARAM_REG, 0x08006000);
	nand_writel(info, NAND_OP_NUM_REG, 0x00);
	nand_writel(info, NAND_CS_RDY_REG, info->csrdy);
}

static int ls_nand_probe(struct udevice *dev)
{
	struct ls_nand_info *info = dev_get_priv(dev);
	struct nand_chip *this;
	struct mtd_info *mtd;
	int ret = 0;
	u32 data;
	int cs = 2;
	static int devnum = 0;

	if (!ofnode_read_u32(dev->node_, "nand-cs-origin", &data))
		cs = data;

	info->cs0 = info->cs = cs;
	info->csrdy = 0x88442200;
	info->chip_cap = 0;

	this = &info->nand_chip;
	mtd = nand_to_mtd(&info->nand_chip);
	mtd->priv = info;
	info->mtd = mtd;

	info->mmio_base = dev_read_addr_ptr(dev);
	if (info->mmio_base == NULL) {
		pr_err("ioremap() failed\n");
		ret = -ENODEV;
		goto fail_mtd;
	}

	ret = dma_get_by_name(dev, "nand-rw", &info->dma_chan);
	if (ret) {
		pr_err("no nand DMA resource defined\n");
		return -ENODEV;
	}

	info->apb_data_addr = (phys_addr_t)info->mmio_base + NAND_DMA_ADDR_REG;
	pr_debug("info->apb_data_addr= 0x%p\n", (void *)info->apb_data_addr);

	ls1x_nand_init_hw(info);
	ret = ls_nand_init_buff(info);
	if (ret)
		goto fail_mtd;

	ls_nand_init_mtd(mtd, info);

	const char *pm = ofnode_read_string(dev->node_, "nand-ecc-algo");
	if (pm) {
		if (!strcmp(pm, "none")) {
			bch = 0;
			this->ecc.mode		= NAND_ECC_NONE;
		} else if (!strcmp(pm, "bch")) {
			if (!ofnode_read_u32(dev->node_, "nand-ecc-strength", &data))
				bch = data;
		} else
			bch = 0;
	}

	ls_nand_init_info(info);

	if (bch) {
#if defined(CONFIG_NAND_ECC_BCH)
		this->ecc.mode		= NAND_ECC_SOFT_BCH;
#else
		this->ecc.mode		= NAND_ECC_SOFT;
		this->ecc.algo		= NAND_ECC_BCH;
#endif
		this->ecc.size = 512;
		this->ecc.strength = bch;
		pr_info("using %u-bit/%u bytes BCH ECC\n", bch, this->ecc.size);
	}

	if (nand_scan(mtd, LS_NAND_MAX_CHIPS)) {
		pr_err("failed to scan nand\n");
		ret = -ENXIO;
		goto fail_free_buf;
	}

	ret = nand_register(devnum++, mtd);
	if (ret)
		return ret;

	return 0;

fail_free_buf:
	free(info->data_buff);
fail_mtd:
	return ret;
}


static const struct udevice_id ls_nand_ids[] = {
	{ .compatible = "loongson,ls-nand", .data = 0 },
	{},
};

U_BOOT_DRIVER(ls_nand) = {
	.name = "ls-nand",
	.id = UCLASS_MTD,
	.of_match = ls_nand_ids,
	.probe = ls_nand_probe,
	.priv_auto	= sizeof(struct ls_nand_info),
};

void board_nand_init(void)
{
	struct udevice *dev;
	int ret;

	ret = uclass_get_device_by_driver(UCLASS_MTD,
					  DM_DRIVER_GET(ls_nand),
					  &dev);
	if (ret && ret != -ENODEV)
		pr_err("Failed to initialize NAND controller. (error %d)\n",
		       ret);
}
