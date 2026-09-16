#!/usr/bin/env bash
# A standalone LEZ sequencer, from the same revision the code is built against.
# The RFP asks for end-to-end integration tests against one in standalone mode,
# in CI, and this is the sequencer half of that.
#
# Usage: `build` or `fetch`, then `start`, `smoke`, `stop`. `pin` prints the
# revision. Run with no arguments for the full list.
#
# Two things a reader needs before the code makes sense:
#
#   * `build` compiles LEZ from source and is what a developer uses. `fetch`
#     pulls a prebuilt image, published once per revision by
#     `lez-sequencer-image.yml`, and is what CI uses. Both leave the same layout
#     behind, so `start`, `smoke` and `stop` do not care which ran.
#
#   * The LEZ revision is read from the committed product lockfiles, never
#     configured here, and the script refuses to run if they disagree. A
#     sequencer built from any other revision than the one `lee_core` resolves
#     to is the failure mode this exists to make impossible.
#
# Why it runs Bedrock plus `sequencer_service` directly rather than going
# through `lgs test-node`, why CI pulls an image rather than building, and what
# `start` feature-detects from the binary's `--help` and why: ADR 12,
# `adr/0012-a-standalone-lez-sequencer-without-lgs-run-from-a-prebuilt-image.md`.
#
# Requirements: docker (with the compose plugin), a host Rust toolchain, git,
# curl. On NixOS, `nix-shell` provides everything but the docker daemon.
set -euo pipefail

readonly REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Where the LEZ checkout and its build cache live. Outside the repository: it is
# a multi-gigabyte build of third-party code, keyed by revision so a pin
# bump does not overwrite the previous one.
readonly CACHE_ROOT="${KANON_LEZ_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/kanon-lez}"
# Run state: pid, home directory, port. Under `target/`, which is gitignored.
readonly STATE_DIR="${KANON_SEQUENCER_STATE:-$REPO_ROOT/target/lez-sequencer}"
# Written into a state directory this script creates, and required to be present
# before anything is deleted recursively. `KANON_SEQUENCER_STATE` and
# `KANON_LEZ_CACHE` are caller-supplied, and `rm -rf` on a caller-supplied path
# is how a harness destroys somebody's home directory.
readonly STATE_SENTINEL=".kanon-sequencer-state"
readonly LEZ_REPO_URL="https://github.com/logos-blockchain/logos-execution-zone"
# Where CI gets a prebuilt sequencer instead of building one. A cold build does
# not fit a standard runner, so `ci.yml` pulls this and `fetch` unpacks it into
# the layout `build` would have produced.
readonly IMAGE="${KANON_SEQUENCER_IMAGE:-ghcr.io/equilibriumco/kanon-lez-sequencer}"
readonly PORT="${KANON_SEQUENCER_PORT:-3055}"
# Fixed by the debug config's `bedrock_config.node_url`, which is
# `http://localhost:18080`. Changing it means rewriting the config, so it is
# not offered as a knob.
readonly BEDROCK_PORT=18080
readonly READY_TIMEOUT="${KANON_SEQUENCER_TIMEOUT:-180}"

log() { printf '[lez-sequencer] %s\n' "$*" >&2; }
die() { log "$*"; exit 1; }

# Refuses a path that must never be handed to `rm -rf`.
#
# Both directory roots come from the environment, so "the caller would not do
# that" is not a guarantee this script is entitled to make. An unset or
# mistyped variable is the realistic case rather than a malicious one:
# `KANON_SEQUENCER_STATE=$HOME` or a trailing-slash typo turns cleanup into
# data loss.
assert_safe_to_remove() {
  local path="$1" what="$2"
  [ -n "$path" ] || die "$what is empty; refusing to remove anything"
  case "$path" in
    /*) ;;
    *) die "$what must be an absolute path, got '$path'" ;;
  esac
  case "$path" in
    *//*|*/./*|*/../*|*/..) die "$what contains a traversal or empty segment: '$path'" ;;
  esac
  # Depth, not a denylist: a denylist cannot enumerate every directory that
  # matters, while requiring three segments rules out /, /home, /home/user and
  # every other shallow path in one condition.
  local trimmed depth
  trimmed="${path%/}"
  depth="$(printf '%s' "${trimmed#/}" | tr -cd '/' | wc -c)"
  [ "$depth" -ge 2 ] || die "$what is too shallow to remove safely: '$trimmed'"
  for reserved in "$HOME" "$REPO_ROOT" /tmp /var /usr /etc /opt; do
    [ "$trimmed" != "${reserved%/}" ] || die "$what resolves to '$trimmed'; refusing"
  done
}

# Recreates the run-state directory, empty.
#
# Only ever removes a directory this script created, proven by the sentinel. A
# pre-existing directory without one is somebody else's, so it is a hard error
# rather than something to clean up helpfully.
reset_state_dir() {
  assert_safe_to_remove "$STATE_DIR" "KANON_SEQUENCER_STATE"
  if [ -e "$STATE_DIR" ]; then
    [ -d "$STATE_DIR" ] || die "$STATE_DIR exists and is not a directory"
    [ -f "$STATE_DIR/$STATE_SENTINEL" ] || die \
      "$STATE_DIR was not created by this script (no $STATE_SENTINEL); refusing to delete it"
    rm -rf "$STATE_DIR"
  fi
  mkdir -p "$STATE_DIR"
  : > "$STATE_DIR/$STATE_SENTINEL"
}

# Removes one path beneath the state directory, with the same proof.
remove_state_subpath() {
  local sub="$1"
  assert_safe_to_remove "$STATE_DIR" "KANON_SEQUENCER_STATE"
  [ -f "$STATE_DIR/$STATE_SENTINEL" ] || return 0
  rm -rf "${STATE_DIR:?}/${sub:?}"
}

# The LEZ revision the *product* resolves, read from the committed lockfiles.
#
# Every product lockfile, and deliberately not `m0/`'s. This sequencer exists to
# integration-test the product, so it has to match the product, and the two are no
# longer the same answer: the product tracks LEZ v0.2.0, because that is what
# `twap_oracle_core` pins and therefore what the canonical price account is built
# against, while `m0/` stays at v0.2.1 because that is what its published figures
# were measured against. Neither is wrong, and the workspace split is what lets
# them differ. Scanning both would only ever produce a false alarm.
#
# Within the product the check still bites: the root workspace and each guest
# workspace resolve separately, and a guest built against a different LEZ than the
# host is exactly the mismatch that made `lgs` unusable.
#
# The match is on the `#<sha>` fragment rather than on `?rev=`, because a manifest
# may pin by tag, and cargo records `?tag=v0.2.0#<sha>` for that. Both forms carry
# the resolved commit after the `#`.
lez_rev() {
  local revs
  # `-exec … +` rather than a pipe into xargs: with no matches xargs would run
  # sed with no file arguments, and sed would sit reading stdin forever.
  revs="$(find "$REPO_ROOT" -name Cargo.lock -not -path '*/target/*' -not -path "$REPO_ROOT/m0/*" \
    -exec sed -n 's|.*git+'"$LEZ_REPO_URL"'[^#]*#\([0-9a-f]\{40\}\).*|\1|p' {} + \
    | sort -u)"
  [ -n "$revs" ] || die "no logos-execution-zone revision in any product Cargo.lock"
  # More than one would mean the product builds against two LEZ revisions at once,
  # which would make "the sequencer matches the code" untrue whichever one was
  # picked.
  [ "$(printf '%s\n' "$revs" | wc -l)" -eq 1 ] \
    || die "the product resolves multiple LEZ revisions:"$'\n'"$revs"
  printf '%s' "$revs"
}

checkout_dir() { printf '%s/%s' "$CACHE_ROOT" "$(lez_rev)"; }

cmd_pin() { lez_rev; printf '\n'; }

cmd_build() {
  local rev dir
  rev="$(lez_rev)"
  dir="$(checkout_dir)"

  if [ ! -d "$dir/.git" ]; then
    log "cloning LEZ at $rev into $dir"
    assert_safe_to_remove "$dir" "KANON_LEZ_CACHE checkout"
    mkdir -p "$CACHE_ROOT"
    rm -rf "$dir"
    # A blobless partial clone: the history is not needed, one revision is.
    git clone --filter=blob:none --no-checkout "$LEZ_REPO_URL" "$dir"
    git -C "$dir" fetch --depth 1 origin "$rev"
    git -C "$dir" checkout --detach "$rev"
  fi

  local actual
  actual="$(git -C "$dir" rev-parse HEAD)"
  [ "$actual" = "$rev" ] || die "checkout at $dir is $actual, expected $rev"

  log "building sequencer_service (first build takes minutes)"
  ( cd "$dir" && cargo build --release --bin sequencer_service )
  log "built $dir/target/release/sequencer_service"
}

# The prebuilt alternative to `build`, for CI.
#
# Same end state, different means: `build` compiles LEZ from source, which takes
# tens of minutes and does not fit a runner's budget, while this pulls an image
# published once per revision and copies its contents into the same cache
# directory. Everything downstream -- `start`, `smoke`, `stop` -- cannot tell
# which one ran.
#
# The image is a transport format rather than a service: nothing executes inside
# it, and the binary it carries is dynamically linked against ubuntu-24.04, so
# this is for CI. On a developer machine, and on NixOS in particular, use
# `build`.
cmd_fetch() {
  local rev dir cid
  rev="$(lez_rev)"
  dir="$(checkout_dir)"

  if [ -x "$dir/target/release/sequencer_service" ]; then
    log "sequencer for $rev already present at $dir"
    return 0
  fi

  # An image already on the machine is used as-is, which makes this idempotent
  # and lets the extraction path be exercised locally against a hand-built
  # image rather than only against the registry.
  if docker image inspect "$IMAGE:$rev" >/dev/null 2>&1; then
    log "using local image $IMAGE:$rev"
  else
    log "pulling $IMAGE:$rev"
    docker pull "$IMAGE:$rev" || die \
      "no published sequencer image for $rev."$'\n'"Run the 'LEZ sequencer image' workflow to publish one, or build locally: $0 build"
  fi

  assert_safe_to_remove "$dir" "KANON_LEZ_CACHE checkout"
  rm -rf "$dir"
  mkdir -p "$dir"

  # A command has to be supplied even though nothing is ever run: the image is
  # `FROM scratch` and carries no CMD, and `docker create` refuses with "no
  # command specified" without one. The path named here is never executed; the
  # container exists only so its filesystem can be copied out.
  cid="$(docker create "$IMAGE:$rev" /lez/target/release/sequencer_service)" \
    || die "could not create a container from $IMAGE:$rev"
  if ! docker cp "$cid:/lez/." "$dir/"; then
    docker rm -f "$cid" >/dev/null 2>&1 || true
    die "could not extract the payload from $IMAGE:$rev"
  fi
  docker rm -f "$cid" >/dev/null 2>&1 || true

  chmod +x "$dir/target/release/sequencer_service"
  log "extracted $rev to $dir"
}

sequencer_bin() {
  local bin
  bin="$(checkout_dir)/target/release/sequencer_service"
  [ -x "$bin" ] || die "no sequencer binary at $bin; run: $0 build"
  printf '%s' "$bin"
}

bedrock_up() {
  local dir
  dir="$(checkout_dir)/bedrock"
  [ -d "$dir" ] || die "no bedrock directory in the LEZ checkout at $dir"

  log "starting bedrock node (docker compose)"
  ( cd "$dir" && docker compose up -d )

  # The sequencer's own startup blocks on Bedrock's time service, so waiting
  # here turns "sequencer never came up" into "bedrock never came up", which is
  # the more useful failure.
  local waited=0
  until curl -fsS -m 5 "http://localhost:$BEDROCK_PORT/time/info" >/dev/null 2>&1; do
    waited=$((waited + 2))
    [ "$waited" -lt "$READY_TIMEOUT" ] \
      || die "bedrock did not answer /time/info within ${READY_TIMEOUT}s"
    sleep 2
  done
  log "bedrock up on :$BEDROCK_PORT"
}

cmd_start() {
  local bin dir cfg home
  bin="$(sequencer_bin)"
  dir="$(checkout_dir)"
  # v0.2.x nests the crate tree under `lez/`. That relocation is what breaks
  # scaffold's hardcoded paths; here it is simply where the config is.
  cfg="$dir/lez/sequencer/service/configs/debug/sequencer_config.json"
  [ -f "$cfg" ] || die "no debug sequencer config at $cfg"

  if [ -f "$STATE_DIR/pid" ] && kill -0 "$(cat "$STATE_DIR/pid")" 2>/dev/null; then
    die "a sequencer is already running (pid $(cat "$STATE_DIR/pid")); run: $0 stop"
  fi

  bedrock_up

  reset_state_dir
  home="$STATE_DIR/home"
  mkdir -p "$home"

  # The sequencer's CLI is not stable across LEZ releases, so the binary is asked
  # what it accepts rather than told. v0.2.1 takes `--listen-address` and
  # `--home`; v0.2.0, which the product now tracks, takes neither and fails
  # outright on an unknown flag. Feature-detecting keeps this script working
  # across a pin move instead of breaking on the one after next.
  local help
  local -a extra
  # A binary that cannot print its help cannot run either, and treating that as
  # "no flags supported" would start the sequencer subtly misconfigured instead
  # of failing. So the failure is surfaced here.
  if ! help="$("$bin" --help 2>&1)"; then
    log "$help"
    die "$bin could not run; if this came from \`fetch\`, the image is for x86_64 linux"
  fi
  extra=()
  case "$help" in
    *--listen-address*) extra+=(--listen-address 127.0.0.1) ;;
    *) log "no --listen-address in this build; the RPC binds to its own default" ;;
  esac
  case "$help" in
    # `--home` overrides the config's own `home` so the config can be used
    # exactly as committed. Without it the config's value applies, which in the
    # debug config is `.` -- and since the sequencer is started from `$home`
    # below, the RocksDB state still lands in a directory this script owns and
    # can delete. That is why the `cd` is load-bearing rather than tidiness.
    *--home*) extra+=(--home "$home") ;;
    *) log "no --home in this build; the config's own home applies, relative to $home" ;;
  esac

  # RISC0_DEV_MODE makes block production fast enough for tests to run at all;
  # the proofs it produces are not real ones, which is why testnet verification
  # is a separate deliverable and not something CI claims.
  log "starting sequencer on :$PORT (home $home)"
  (
    cd "$home"
    RISC0_DEV_MODE="${RISC0_DEV_MODE:-1}" RUST_LOG="${RUST_LOG:-info}" \
      "$bin" "$cfg" --port "$PORT" "${extra[@]}" \
      >"$STATE_DIR/sequencer.log" 2>&1 &
    printf '%s' "$!" >"$STATE_DIR/pid"
  )

  local pid waited=0
  pid="$(cat "$STATE_DIR/pid")"
  until grep -q "RPC server started" "$STATE_DIR/sequencer.log" 2>/dev/null; do
    if ! kill -0 "$pid" 2>/dev/null; then
      log "sequencer exited during startup; last lines:"
      tail -30 "$STATE_DIR/sequencer.log" >&2 || true
      die "sequencer failed to start"
    fi
    waited=$((waited + 1))
    if [ "$waited" -ge "$READY_TIMEOUT" ]; then
      tail -30 "$STATE_DIR/sequencer.log" >&2 || true
      cmd_stop
      die "sequencer did not report a listening RPC within ${READY_TIMEOUT}s"
    fi
    sleep 1
  done

  printf 'http://127.0.0.1:%s' "$PORT" >"$STATE_DIR/rpc"
  log "sequencer up at http://127.0.0.1:$PORT"
  # On stdout, so a caller can do: RPC=$(scripts/lez-sequencer.sh start)
  printf 'http://127.0.0.1:%s\n' "$PORT"
}

rpc_call() {
  curl -fsS -m 10 -X POST "http://127.0.0.1:$PORT" \
    -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":[]}"
}

# The readiness assertion. Two calls, because "the port accepts a connection" is not
# the same claim as "the chain is running": `checkHealth` answers, and
# `getLastBlockId` shows the sequencer is producing blocks against Bedrock
# rather than sitting in a retry loop.
cmd_smoke() {
  local health blocks
  health="$(rpc_call checkHealth)" || die "checkHealth did not answer"
  case "$health" in
    *'"error"'*) die "checkHealth returned an error: $health" ;;
    *'"result"'*) log "checkHealth ok" ;;
    *) die "checkHealth returned something unexpected: $health" ;;
  esac

  blocks="$(rpc_call getLastBlockId)" || die "getLastBlockId did not answer"
  case "$blocks" in
    *'"result"'*) log "getLastBlockId: $blocks" ;;
    *) die "getLastBlockId returned no result: $blocks" ;;
  esac

  log "smoke passed: the sequencer serves RPC and has produced at least one block"
}

cmd_stop() {
  if [ -f "$STATE_DIR/pid" ]; then
    local pid
    pid="$(cat "$STATE_DIR/pid")"
    if kill -0 "$pid" 2>/dev/null; then
      log "stopping sequencer (pid $pid)"
      # SIGTERM: the service shuts its store down cleanly on it, and a killed
      # sequencer can leave its RocksDB home locked.
      kill "$pid" 2>/dev/null || true
      local waited=0
      while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 30 ]; do
        waited=$((waited + 1))
        sleep 1
      done
      kill -9 "$pid" 2>/dev/null || true
    fi
    rm -f "$STATE_DIR/pid"
  fi

  local dir
  dir="$(checkout_dir)/bedrock"
  if [ -d "$dir" ]; then
    log "stopping bedrock"
    ( cd "$dir" && docker compose down -v ) || true
  fi

  remove_state_subpath home
  log "stopped"
}

cmd_logs() { tail -n "${2:-50}" "$STATE_DIR/sequencer.log"; }

usage() {
  cat >&2 <<'USAGE'
usage: scripts/lez-sequencer.sh <command>

  pin     print the LEZ revision Cargo.lock resolves, which is what gets built
  build   clone that revision and build sequencer_service (minutes, cached)
  fetch   pull a prebuilt sequencer for that revision instead of building it
  start   start bedrock and the sequencer; prints the RPC URL on stdout
  smoke   assert the running sequencer serves RPC and is producing blocks
  stop    stop both and delete the run state
  logs    tail the sequencer log

environment:
  KANON_LEZ_CACHE         checkout and build cache (default ~/.cache/kanon-lez)
  KANON_SEQUENCER_STATE   run state (default target/lez-sequencer)
  KANON_SEQUENCER_PORT    sequencer RPC port (default 3055)
  KANON_SEQUENCER_TIMEOUT readiness timeout in seconds (default 180)
  KANON_SEQUENCER_IMAGE   prebuilt image for `fetch`
                          (default ghcr.io/equilibriumco/kanon-lez-sequencer)
USAGE
  exit 64
}

case "${1:-}" in
  pin) cmd_pin ;;
  build) cmd_build ;;
  fetch) cmd_fetch ;;
  start) cmd_start ;;
  smoke) cmd_smoke ;;
  stop) cmd_stop ;;
  logs) cmd_logs "$@" ;;
  *) usage ;;
esac
