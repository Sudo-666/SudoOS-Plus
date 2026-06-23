#!/usr/bin/env bash
set -euo pipefail
cat <<'EOF'
Local official-style evaluation skeleton:

  git clone https://github.com/oscomp/autotest-for-oskernel.git /tmp/autotest-for-oskernel
  mkdir -p /tmp/oscomp-data
  cp -rf /tmp/autotest-for-oskernel/kernel/judge/* /tmp/oscomp-data/
  cd /tmp/oscomp-data
  wget https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-la.img.xz
  wget https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-rv.img.xz
  unxz sdcard-la.img.xz && gzip sdcard-la.img
  unxz sdcard-rv.img.xz && gzip sdcard-rv.img
  cd /tmp/autotest-for-oskernel/kernel && zip ../kernel.zip -r *

  docker run --rm \
    -v "$PWD":/coursegrader/submit \
    -v /tmp/oscomp-data:/coursegrader/testdata \
    -v /tmp/autotest-for-oskernel:/cg \
    -v /tmp/oscomp-data:/mnt/cghook/ \
    zhouzhouyi/os-contest:20260510 python3 /cg/kernel.zip
EOF
