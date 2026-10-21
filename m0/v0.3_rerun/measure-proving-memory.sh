#!/usr/bin/env bash
# Peak memory of a succinct proof, per guest configuration.
#
# Two numbers per run, because M0's claim was about VRAM and this machine may or
# may not be proving on the GPU:
#
#   * peak resident set size summed across the prover and every descendant,
#     sampled from /proc every 200 ms — system RAM. (`/usr/bin/time -v` would be
#     the obvious tool; NixOS has no /usr/bin/time.)
#   * peak GPU memory in use, sampled on the same interval, against the idle
#     baseline so a desktop's own usage is not charged to the prover.
#
# Reported separately rather than summed: if the GPU figure never rises above
# baseline, the prover never touched it and only the RAM number means anything.
set -o errexit
set -o nounset
set -o pipefail

PROBE_DIR="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
OUT="${2:-/tmp/proving-memory}"
mkdir -p "$OUT"

gpu_now() {
  nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits 2>/dev/null || echo 0
}

# Total RSS in kB of every lez-probe / r0vm process alive right now.
rss_now() {
  local total=0 rss
  for pid in $(pgrep -f 'lez-probe|r0vm' 2>/dev/null); do
    rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
    total=$(( total + ${rss:-0} ))
  done
  echo "$total"
}

BASELINE=$(gpu_now)
echo "GPU total: $(nvidia-smi --query-gpu=memory.total --format=csv,noheader 2>/dev/null || echo 'none')"
echo "GPU idle baseline: ${BASELINE} MiB (desktop and other processes)"
echo

# Build once, so compilation is not inside the measured window.
cargo build --release --quiet --manifest-path "$PROBE_DIR/Cargo.toml" -p lez-probe

for VARIANT in mixed accelerated; do
  echo "=== $VARIANT ==="

  ( while true; do echo "$(rss_now) $(gpu_now)"; sleep 0.2; done ) > "$OUT/$VARIANT.samples" 2>/dev/null &
  SAMPLER=$!

  "$PROBE_DIR/target/release/lez-probe" --signers 3 --succinct --only "$VARIANT" \
    > "$OUT/$VARIANT.out" 2>&1 || true

  kill "$SAMPLER" 2>/dev/null || true
  wait "$SAMPLER" 2>/dev/null || true

  PEAK_RSS=$(awk '{print $1}' "$OUT/$VARIANT.samples" | sort -n | tail -1)
  PEAK_GPU=$(awk '{print $2}' "$OUT/$VARIANT.samples" | sort -n | tail -1)

  grep -E "succinct:" "$OUT/$VARIANT.out" || cat "$OUT/$VARIANT.out"
  printf '  peak RSS:  %s kB (%.2f GiB)\n' "$PEAK_RSS" "$(echo "$PEAK_RSS" | awk '{print $1/1048576}')"
  printf '  peak VRAM: %s MiB total, %s MiB above baseline\n' "$PEAK_GPU" "$(( PEAK_GPU - BASELINE ))"
  echo
done
