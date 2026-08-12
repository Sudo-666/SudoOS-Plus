# SudoOS VisionFive 2 TFTP boot script (network-address-free).
#
# Load it from U-Boot (first configure the network manually, e.g. via router
# dhcp or direct link; do NOT saveenv on the first round):
#   tftpboot ${scriptaddr} sudoos/vf2/sudoos-vf2-tftp.scr
#   source ${scriptaddr}
#
# Then, once stable:
#   setenv sudoos_conf conf-smp
#   setenv sudoos_tftp 'tftpboot 0x60000000 sudoos/vf2/sudoos-visionfive2.itb && iminfo 0x60000000 && bootm 0x60000000#${sudoos_conf}'
#   run sudoos_tftp
#
# The script itself does not hardcode IP addresses.

if test -z "$serverip"; then
    echo "error: serverip is not set; run 'dhcp' or 'setenv serverip <host>' first"
    exit 1
fi

if test -z "$sudoos_conf"; then
    setenv sudoos_conf conf-smp
fi

setenv sudoos_fit sudoos/vf2/sudoos-visionfive2.itb

echo "SudoOS: TFTP ${sudoos_fit} to 0x60000000, bootm #${sudoos_conf}"
tftpboot 0x60000000 ${sudoos_fit}

iminfo 0x60000000

# Clear a stale U-Boot env bootargs (e.g. the stock Linux command line) so the
# FIT DTB's /chosen/bootargs (rdinit=/init, sudoos.maxcpus=N) takes effect for
# the selected config. Without this, bootm would hand the board's env bootargs
# to the kernel and conf-single would boot 4 cores onto the selftest path.
setenv bootargs

bootm 0x60000000#${sudoos_conf}
