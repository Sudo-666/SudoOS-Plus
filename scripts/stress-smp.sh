#!/usr/bin/env bash

set -Eeuo pipefail

SCRIPT_DIR="$(
    cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
    pwd
)"

ROOT_DIR="$(
    cd -- "${SCRIPT_DIR}/.."
    pwd
)"

ARCHES="${STRESS_ARCHES:-${ARCH:-riscv64}}"
SMPS="${STRESS_SMPS:-${SMP:-4}}"
MEMS="${STRESS_MEMS:-${MEM:-256M}}"
PROFILES="${STRESS_PROFILES:-${PROFILE:-debug}}"
LOOPS="${STRESS_LOOPS:-1}"
TIMEOUT="${STRESS_TIMEOUT:-${SMP_SMOKE_TIMEOUT:-75}}"
LOG_ROOT="${STRESS_LOG_DIR:-${ROOT_DIR}/build/stress-smp}"
SEED="${STRESS_SEED:-$(date +%Y%m%d%H%M%S)}"

die() {
    echo "stress-smp: error: $*" >&2
    exit 1
}

[[ "${LOOPS}" =~ ^[0-9]+$ ]] || die "STRESS_LOOPS must be a positive integer"
(( LOOPS > 0 )) || die "STRESS_LOOPS must be greater than zero"

mkdir -p "${LOG_ROOT}"

echo "SudoOS SMP stress"
echo "  seed     : ${SEED}"
echo "  arches   : ${ARCHES}"
echo "  smps     : ${SMPS}"
echo "  mems     : ${MEMS}"
echo "  profiles : ${PROFILES}"
echo "  loops    : ${LOOPS}"
echo "  timeout  : ${TIMEOUT}s"
echo "  log dir  : ${LOG_ROOT}"
echo

run_one() {
    local arch="$1"
    local smp="$2"
    local mem="$3"
    local profile="$4"
    local loop="$5"
    local case_name="${arch}-smp${smp}-${mem}-${profile}-loop${loop}-seed${SEED}"
    local case_dir="${LOG_ROOT}/${case_name}"
    local smoke_log="${ROOT_DIR}/build/${arch}/smoke.log"

    mkdir -p "${case_dir}"

    cat > "${case_dir}/config.txt" <<EOF
seed=${SEED}
arch=${arch}
smp=${smp}
mem=${mem}
profile=${profile}
loop=${loop}
timeout=${TIMEOUT}
qemu_args=${QEMU_ARGS:-}
EOF

    echo "==> ${case_name}"
    if ARCH="${arch}" \
        SMP="${smp}" \
        MEM="${mem}" \
        PROFILE="${profile}" \
        "${ROOT_DIR}/scripts/smoke.py" \
            --arch "${arch}" \
            --profile "${profile}" \
            --timeout "${TIMEOUT}"
    then
        if [[ -f "${smoke_log}" ]]; then
            cp "${smoke_log}" "${case_dir}/serial.log"
        fi
        echo "PASS ${case_name}"
        echo
        return 0
    fi

    if [[ -f "${smoke_log}" ]]; then
        cp "${smoke_log}" "${case_dir}/serial.log"
    fi

    echo "FAIL ${case_name}" >&2
    echo "config: ${case_dir}/config.txt" >&2
    echo "serial: ${case_dir}/serial.log" >&2
    return 1
}

for loop in $(seq 1 "${LOOPS}"); do
    for profile in ${PROFILES}; do
        for mem in ${MEMS}; do
            for smp in ${SMPS}; do
                for arch in ${ARCHES}; do
                    run_one "${arch}" "${smp}" "${mem}" "${profile}" "${loop}"
                done
            done
        done
    done
done

echo "stress-smp: PASS"
