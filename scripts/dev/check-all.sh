#!/usr/bin/env bash

set -euo pipefail

readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
cd "${ROOT_DIR}"

usage() {
    cat <<'EOF'
Usage: scripts/dev/check-all.sh [options]

Runs the TLC verification workflow in a deterministic sequence:
  1. cargo fmt --all --check
  2. cargo clippy --workspace --all-targets --all-features -- -D warnings
  3. cargo test --workspace --all-targets
  4. cargo audit --deny warnings
  5. tlc parity harness
  6. performance validation hook

Options:
      --config <path>   Load environment overrides from the given file.
      --skip-parity     Skip the parity harness step.
      --skip-perf       Skip the performance step.
      --skip-audit      Skip the cargo audit step.
  -h, --help            Show this help text.

Environment overrides (may also live in the config file):
  SKIP_PARITY=1         Skip the parity harness.
  SKIP_PERF=1           Skip performance suite.
  SKIP_AUDIT=1          Skip cargo-audit.
  TLC_PARITY_LEGACY     Path to legacy TLC launcher (default: scripts/dev/legacy-tlc.sh).
  TLC_PARITY_SPECS_DIR  Path to parity regression suite (default: tests/golden/regression-suite).
  TLC_PARITY_OUTPUT_DIR Output directory for parity artifacts (default: artifacts/parity).
  TLC_PARITY_FILTER     Optional glob filter forwarded to the parity harness.
  CHECK_ALL_PERF_CMD    Shell command invoked for the performance step (if unset a placeholder runs).

Set CHECK_ALL_CONFIG to point at a config file, or pass --config.
EOF
}

log_step() {
    printf '\n==> %s\n' "$1"
}

log_info() {
    printf '    %s\n' "$1"
}

die() {
    printf 'error: %s\n' "$1" >&2
    exit 1
}

ensure_tool() {
    local tool="$1"
    if ! command -v "${tool}" >/dev/null 2>&1; then
        die "required tool '${tool}' not found in PATH"
    fi
}

resolve_path() {
    local raw="$1"
    if [[ -z "${raw}" ]]; then
        printf '\n' ""
        return
    fi
    if [[ "${raw}" = /* ]]; then
        printf '%s\n' "${raw}"
    else
        printf '%s\n' "${ROOT_DIR}/${raw}"
    fi
}

run_cmd() {
    log_info "running: $*"
    "$@"
}

# Discover configuration file before applying option overrides.
CONFIG_PATH=""
if [[ -n "${CHECK_ALL_CONFIG:-}" ]]; then
    CONFIG_PATH="${CHECK_ALL_CONFIG}"
fi

args=("$@")
index=0
while [[ ${index} -lt ${#args[@]} ]]; do
    case "${args[${index}]}" in
        --config)
            next=$((index + 1))
            if [[ ${next} -ge ${#args[@]} ]]; then
                die "--config requires a value"
            fi
            CONFIG_PATH="${args[${next}]}"
            index=$((index + 1))
            ;;
    esac
    index=$((index + 1))
done

if [[ -z "${CONFIG_PATH}" ]]; then
    DEFAULT_CONFIG="${ROOT_DIR}/scripts/dev/check-all.env"
    if [[ -f "${DEFAULT_CONFIG}" ]]; then
        CONFIG_PATH="${DEFAULT_CONFIG}"
    fi
fi

if [[ -n "${CONFIG_PATH}" ]]; then
    if [[ ! -f "${CONFIG_PATH}" ]]; then
        die "config file '${CONFIG_PATH}' not found"
    fi
    # shellcheck disable=SC1090
    source "${CONFIG_PATH}"
fi

SKIP_PARITY="${SKIP_PARITY:-0}"
SKIP_PERF="${SKIP_PERF:-0}"
SKIP_AUDIT="${SKIP_AUDIT:-0}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-parity)
            SKIP_PARITY=1
            ;;
        --skip-perf)
            SKIP_PERF=1
            ;;
        --skip-audit)
            SKIP_AUDIT=1
            ;;
        --config)
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            usage >&2
            die "unknown option '$1'"
            ;;
    esac
    shift
done

TEMP_LOCK_CREATED=0
cleanup() {
    if [[ "${TEMP_LOCK_CREATED}" -eq 1 && -f "${ROOT_DIR}/Cargo.lock" ]]; then
        rm -f "${ROOT_DIR}/Cargo.lock"
        log_info "removed temporary Cargo.lock"
    fi
}
trap cleanup EXIT

run_fmt() {
    log_step "cargo fmt"
    run_cmd cargo fmt --all --check
}

run_clippy() {
    log_step "cargo clippy"
    run_cmd cargo clippy --workspace --all-targets --all-features -- -D warnings
}

run_tests() {
    log_step "cargo test"
    run_cmd cargo test --workspace --all-targets
}

run_audit() {
    if [[ "${SKIP_AUDIT}" -eq 1 ]]; then
        log_step "cargo audit (skipped)"
        log_info "SKIP_AUDIT=1 - skipping vulnerability audit."
        return
    fi

    log_step "cargo audit"
    if ! command -v cargo-audit >/dev/null 2>&1; then
        die "cargo-audit is required; install with 'cargo install cargo-audit'"
    fi

    if [[ ! -f "${ROOT_DIR}/Cargo.lock" ]]; then
        log_info "generating temporary Cargo.lock"
        cargo generate-lockfile >/dev/null
        TEMP_LOCK_CREATED=1
    else
        log_info "using existing Cargo.lock"
    fi

    run_cmd cargo audit --deny warnings
}

run_parity() {
    if [[ "${SKIP_PARITY}" -eq 1 ]]; then
        log_step "parity harness (skipped)"
        log_info "SKIP_PARITY=1 - skipping parity harness."
        return
    fi

    log_step "parity harness"
    local legacy="${TLC_PARITY_LEGACY:-${ROOT_DIR}/scripts/dev/legacy-tlc.sh}"
    local specs_dir="${TLC_PARITY_SPECS_DIR:-${ROOT_DIR}/tests/golden/regression-suite}"
    local output_dir="${TLC_PARITY_OUTPUT_DIR:-${ROOT_DIR}/artifacts/parity}"
    local filter="${TLC_PARITY_FILTER:-}"

    specs_dir="$(resolve_path "${specs_dir}")"
    output_dir="$(resolve_path "${output_dir}")"
    legacy="$(resolve_path "${legacy}")"

    if [[ ! -d "${specs_dir}" ]]; then
        die "parity specs directory '${specs_dir}' does not exist; configure TLC_PARITY_SPECS_DIR or set SKIP_PARITY=1"
    fi

    mkdir -p "${output_dir}"

    cmd=(cargo run -p tlc-parity -- --legacy "${legacy}" --specs "${specs_dir}" --output "${output_dir}")
    if [[ -n "${filter}" ]]; then
        cmd+=("--filter" "${filter}")
    fi
    if [[ -n "${TLC_PARITY_WORKERS:-}" ]]; then
        cmd+=("--workers" "${TLC_PARITY_WORKERS}")
    fi
    if [[ -n "${TLC_PARITY_GOLDEN_CACHE:-}" ]]; then
        cmd+=("--golden-cache" "$(resolve_path "${TLC_PARITY_GOLDEN_CACHE}")")
    fi

    run_cmd "${cmd[@]}"
}

run_perf() {
    if [[ "${SKIP_PERF}" -eq 1 ]]; then
        log_step "performance suite (skipped)"
        log_info "SKIP_PERF=1 - skipping performance validation."
        return
    fi

    log_step "performance suite"
    if [[ -n "${CHECK_ALL_PERF_CMD:-}" ]]; then
        run_cmd bash -lc "${CHECK_ALL_PERF_CMD}"
        return
    fi

    local perf_runner="${ROOT_DIR}/scripts/dev/run-perf.sh"
    if [[ -x "${perf_runner}" ]]; then
        run_cmd "${perf_runner}"
        return
    fi

    log_info "no performance harness configured; placeholder step completed."
    log_info "set CHECK_ALL_PERF_CMD or add scripts/dev/run-perf.sh to execute real benchmarks."
}

main() {
    ensure_tool cargo
    run_fmt
    run_clippy
    run_tests
    run_audit
    run_parity
    run_perf
    log_step "check-all completed successfully"
}

main "$@"
