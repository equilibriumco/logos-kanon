#!/usr/bin/env bash
# A standalone LEZ sequencer, from the same revision the code is built against.
# The RFP asks for end-to-end integration tests against one in standalone mode,
# in CI, and this is the sequencer half of that.
#
# Usage: `build` or `fetch`, then `start`, `smoke`, `stop`. `pin` prints the
# revision. Run with no arguments for the full list.
#
# Three things a reader needs before the code makes sense:
#
#   * `build` compiles LEZ from source and is what a developer uses. `fetch`
#     pulls a prebuilt image, published once per revision by
#     `lez-sequencer-image.yml`, and is what CI uses. Both leave the same layout
#     behind, so `start`, `smoke` and `stop` do not care which ran.
#
#   * `start` runs the sequencer **in a container** by default, beside the
#     Bedrock node that was already one. `KANON_SEQUENCER_RUNTIME=host` runs the
#     binary as a host process instead, which is the only thing that works where
#     the published image's glibc does not -- NixOS, or any host that is not
#     x86_64 linux, where `build` is the way in. ADR 34 records the switch and
#     what host networking costs; ADR 12 is the decision it supersedes in part.
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
# Which of the two `start` paths runs. `container` is the default because it is
# the one that does not depend on the host's libc matching the image's; `host`
# is for a machine where a glibc binary will not run, and is what `build`
# produces for.
readonly RUNTIME="${KANON_SEQUENCER_RUNTIME:-container}"
# What shape of image `start` needs, independent of the LEZ revision the tag
# names. The tag answers "which sequencer"; this answers "built how", and the two
# move for different reasons -- a Dockerfile change, a package added to the apt
# list, a new path inside the payload, all at one unchanged LEZ revision. Without
# it the entrypoint check only ever catches the one transition it was written
# for, and every later change silently reuses a stale image.
#
# The script owns the number and `format` prints it, the way `pin` prints the
# revision, so `lez-sequencer-image.yml` stamps what this file asks for rather
# than carrying a second copy to keep in step.
readonly IMAGE_FORMAT=1
# The label it is stamped as, on both sides.
readonly FORMAT_LABEL=co.equilibrium.kanon.image-format
# Marks a container this script started, so a name collision with something
# unrelated is not something `start` will delete.
readonly OWNER_LABEL=co.equilibrium.kanon.sequencer
# Named rather than left to docker so `stop` can find it after a shell has gone
# away, and so two checkouts on one machine collide loudly rather than silently
# sharing a port.
readonly CONTAINER="${KANON_SEQUENCER_CONTAINER:-kanon-lez-sequencer}"
readonly PORT="${KANON_SEQUENCER_PORT:-3055}"
# Where the image keeps the config the sequencer is pointed at. The image's own
# copy of the file `fetch` extracts, so the two runtimes read the same bytes --
# LEZ's committed debug config, unmodified, which is the property ADR 12 chose
# host networking to keep.
readonly CONTAINER_CONFIG=/lez/lez/sequencer/service/configs/debug/sequencer_config.json
# The image's WORKDIR, and where the run state is bind-mounted. Named because
# `--home` has to be given the path as the *sequencer* sees it, not as the host
# does.
readonly CONTAINER_HOME=/var/lib/lez
# The host-side config path and binary, resolved in `cmd_start` once the checkout
# is known. The binary is resolved eagerly there rather than inside the two
# functions that need it: `sequencer_bin` reports a missing build by dying, and
# a `die` inside a command substitution only kills the subshell -- the caller
# would go on to run `--help` with an empty command and report that instead.
HOST_CONFIG=""
HOST_BIN=""
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

cmd_format() { printf '%s\n' "$IMAGE_FORMAT"; }

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
image_ref() { printf '%s:%s' "$IMAGE" "$(lez_rev)"; }

# The image for the pinned revision, on this machine.
#
# An image already present is used as-is, which makes both callers idempotent and
# lets the container path be exercised locally against a hand-built image rather
# than only against the registry.
ensure_image() {
  local ref
  ref="$(image_ref)"
  if docker image inspect "$ref" >/dev/null 2>&1; then
    log "using local image $ref"
  else
    log "pulling $ref"
    docker pull "$ref" || die \
      "no published sequencer image for $(lez_rev)."$'\n'"Run the 'LEZ sequencer image' workflow to publish one, or run it here as a host process:"$'\n'"  $0 build"$'\n'"  KANON_SEQUENCER_RUNTIME=host $0 start"
  fi
}

# That the image is one the container runtime can actually run.
#
# Every image published before ADR 34 is `FROM scratch` with no entrypoint.
# `fetch` still works against one -- extraction does not care -- so this is
# asserted by `start` and not by `ensure_image`, which both share. Without it the
# failure is `docker run` complaining about a missing executable, which reads
# like a broken script rather than like an image that needs republishing.
assert_image_runnable() {
  local ref entrypoint format
  ref="$(image_ref)"
  # An inspect that fails is not an image that passes. Without this the
  # substitution yields the empty string, `[ "" = 0 ]` is false, and the
  # function reports "runnable" for an image it could not read at all -- so a
  # pruned image or a daemon hiccup would come back as the `docker run` error
  # this check exists to translate.
  entrypoint="$(docker image inspect -f '{{len .Config.Entrypoint}}' "$ref" 2>&1)" \
    || die "could not inspect $ref: $entrypoint"
  if [ "$entrypoint" = 0 ]; then
    die "$ref carries no entrypoint, so it predates adr/0034 and cannot be run."$'\n'"Re-run the 'LEZ sequencer image' workflow to republish it, or: KANON_SEQUENCER_RUNTIME=host $0 start"
  fi

  # The entrypoint check above only ever catches the one transition it was
  # written for. The tag is the LEZ revision, so a Dockerfile change at an
  # unchanged revision -- a package added to the apt list, a path moved inside
  # the payload -- produces an image this check would wave through, and a
  # developer holding the older tag never pulls the replacement because
  # `ensure_image` uses a local image as-is. The format label is what makes those
  # visible.
  format="$(image_format "$ref")"
  if [ "$format" != "$IMAGE_FORMAT" ]; then
    # A stale *local* image is the likely cause, not a stale published one.
    # `ensure_image` uses whatever is on the machine as-is, so someone who pulled
    # before the format moved keeps meeting this error while the registry has
    # had the answer all along -- and re-running the publish workflow does
    # nothing for them, because it republishes something they never fetch. So
    # the tag is refreshed once and re-read before this is called a failure.
    log "$ref is image format '${format:-<none>}', not $IMAGE_FORMAT; refreshing from the registry"
    docker pull "$ref" >/dev/null 2>&1 || true
    format="$(image_format "$ref")"
  fi
  [ "$format" = "$IMAGE_FORMAT" ] || die \
    "$ref is image format '${format:-<none>}' and this script needs $IMAGE_FORMAT, after a refresh."$'\n'"Re-run the 'LEZ sequencer image' workflow to republish it, or: KANON_SEQUENCER_RUNTIME=host $0 start"
}

# The format label on an image, or the empty string.
image_format() {
  docker image inspect -f "{{index .Config.Labels \"$FORMAT_LABEL\"}}" "$1" 2>/dev/null || true
}

cmd_fetch() {
  local rev dir cid
  rev="$(lez_rev)"
  dir="$(checkout_dir)"

  if [ -x "$dir/target/release/sequencer_service" ]; then
    log "sequencer for $rev already present at $dir"
    return 0
  fi

  ensure_image

  assert_safe_to_remove "$dir" "KANON_LEZ_CACHE checkout"
  rm -rf "$dir"
  mkdir -p "$dir"

  # The container is created and never started: it exists only so its filesystem
  # can be copied out. The argument is left in place from when the image was
  # `FROM scratch` and `docker create` refused without one; the image has an
  # entrypoint of its own now, so it is redundant rather than load-bearing, and
  # harmless either way because nothing here runs.
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
  # Wanted in both runtimes: the compose file is LEZ's own and has to be on the
  # host for `docker compose` to read it, so even the container path needs a
  # checkout. `fetch` is the cheap way to get one.
  [ -d "$dir" ] || die \
    "no bedrock directory in the LEZ checkout at $dir; run: $0 fetch (or $0 build)"

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

# Whether a sequencer this script started is still alive, in either shape.
#
# Both are checked whichever runtime is selected, and that is what makes the
# "already running" refusal in `cmd_start` useful: a host sequencer left over
# from an earlier run holds `$PORT`, and a container started against it would
# fail to bind with an error a reader has to go into the log to find. `start`
# wipes the state directory, so the pid file cannot be stale by the time the
# readiness loop below asks the same question.
sequencer_running() {
  local id
  id="$(started_container)" || true
  if [ -n "$id" ] \
    && [ "$(docker inspect -f '{{.State.Running}}' "$id" 2>/dev/null)" = true ]; then
    return 0
  fi
  [ -f "$STATE_DIR/pid" ] && kill -0 "$(cat "$STATE_DIR/pid")" 2>/dev/null
}

# Brings `$STATE_DIR/sequencer.log` up to date.
#
# A host process writes there itself, so this is a no-op for it. A container's
# output lives in docker's own log, and copying it out each time round the loop
# is what lets one `grep` serve both runtimes -- and leaves a file behind that
# `logs` can read after the container is gone.
refresh_log() {
  [ "$RUNTIME" = container ] || return 0
  local id
  id="$(started_container)" || true
  [ -n "$id" ] || return 0
  docker logs "$id" >"$STATE_DIR/sequencer.log" 2>&1 || true
}

# The sequencer's `--help`, from whichever artefact is about to be run.
#
# The CLI is not stable across LEZ releases, so the sequencer is asked what it
# accepts rather than told. v0.2.1 takes `--listen-address` and `--home`; v0.2.0,
# which the product now tracks, takes neither and fails outright on an unknown
# flag. Feature-detecting keeps this script working across a pin move instead of
# breaking on the one after next.
#
# A sequencer that cannot print its help cannot run either, and treating that as
# "no flags supported" would start it subtly misconfigured instead of failing, so
# both callers below turn a failure here into an error.
sequencer_help() {
  if [ "$RUNTIME" = container ]; then
    docker run --rm "$(image_ref)" --help 2>&1
  else
    "$HOST_BIN" --help 2>&1
  fi
}

# The sequencer in a container, beside the Bedrock node that was already one.
#
# `--network host` is what lets the committed config be used exactly as it
# ships. `bedrock_config.node_url` is `http://localhost:18080`, and in a bridge
# network `localhost` is the container itself, so any other arrangement means
# rewriting a field of LEZ's own config -- which is the thing ADR 12 set out not
# to do. It also puts the RPC straight onto the host's `$PORT`, so nothing
# downstream has to know a container is involved. It needs no setting up on
# linux; on Docker Desktop it needs 4.34 or later with host networking enabled
# in Settings -> Resources -> Network, which is off by default.
# `KANON_SEQUENCER_RUNTIME=host` is the way out either way, and ADR 34 records
# the alternative that was weighed against this.
#
# `--user` matters more than it looks: RocksDB writes into the bind mount below,
# and a root-owned state directory is one `stop` cannot delete, under a `target/`
# the developer owns.
start_container() {
  local home
  home="$STATE_DIR/home"

  log "starting sequencer container $CONTAINER on :$PORT (home $home)"
  # The id is captured and recorded, and everything afterwards addresses the
  # container by it. A name is a caller's variable: `KANON_SEQUENCER_CONTAINER=foo
  # start` followed by a plain `stop` would look up the default name, find
  # nothing, conclude nothing was running and delete the state directory while
  # `foo` still had it mounted. The id is what this run actually created.
  local id
  id="$(docker run --detach --name "$CONTAINER" \
    --label "$OWNER_LABEL=1" \
    --network host \
    --user "$(id -u):$(id -g)" \
    --volume "$home:$CONTAINER_HOME" \
    --env RISC0_DEV_MODE="${RISC0_DEV_MODE:-1}" \
    --env RUST_LOG="${RUST_LOG:-info}" \
    "$(image_ref)" \
    "$CONTAINER_CONFIG" --port "$PORT" "$@")" \
    || die "could not start $CONTAINER from $(image_ref)"
  printf '%s' "$id" >"$STATE_DIR/container"
}

# Whether a container exists, distinguishing "no" from "could not tell".
#
# `docker inspect` exits non-zero both for a container that is genuinely absent
# and for a daemon that would not answer, and the two must not be treated alike:
# reading the second as the first is how `stop` concludes there is nothing left
# and deletes a live sequencer's database. Prints `present`, `absent`, or
# `unknown`, and every caller here fails closed on the third.
container_state() {
  local id="$1" out
  if out="$(docker inspect "$id" 2>&1)"; then
    printf 'present'
  elif printf '%s' "$out" | grep -qi 'no such object'; then
    printf 'absent'
  else
    printf 'unknown'
  fi
}

# The container this run started, or nothing.
#
# Read from the state directory rather than derived from `$CONTAINER`, for the
# reason `start_container` gives. Empty output means there is nothing of ours to
# act on, which is the safe answer for every caller below.
started_container() {
  [ -f "$STATE_DIR/container" ] && cat "$STATE_DIR/container"
}

# The sequencer as a host process, from a binary `build` or `fetch` left behind.
start_host() {
  local home
  home="$STATE_DIR/home"

  log "starting sequencer on :$PORT (home $home)"
  (
    cd "$home"
    RISC0_DEV_MODE="${RISC0_DEV_MODE:-1}" RUST_LOG="${RUST_LOG:-info}" \
      "$HOST_BIN" "$HOST_CONFIG" --port "$PORT" "$@" \
      >"$STATE_DIR/sequencer.log" 2>&1 &
    printf '%s' "$!" >"$STATE_DIR/pid"
  )
}

cmd_start() {
  local dir home guest_home previous actual

  case "$RUNTIME" in
    container|host) ;;
    *) die "KANON_SEQUENCER_RUNTIME is '$RUNTIME'; expected 'container' or 'host'" ;;
  esac

  dir="$(checkout_dir)"
  # v0.2.x nests the crate tree under `lez/`. That relocation is what breaks
  # scaffold's hardcoded paths; here it is simply where the config is. The
  # container reads the same file from inside the image, which is why the two
  # paths differ by a prefix and not by content.
  HOST_CONFIG="$dir/lez/sequencer/service/configs/debug/sequencer_config.json"

  if sequencer_running; then
    die "a sequencer is already running; run: $0 stop"
  fi
  # A stopped container of the same name still owns the name, so `docker run`
  # below would refuse. Removing one that is not running is what makes `start`
  # after a crash work without a manual `stop` -- but only if it is one of ours.
  # `KANON_SEQUENCER_CONTAINER` is a caller's variable and could name anything on
  # the machine, so the label decides, and a collision with something else is
  # reported rather than deleted.
  if [ "$RUNTIME" = container ]; then
    case "$(container_state "$CONTAINER")" in
      absent) ;;
      unknown) die "a container named $CONTAINER may exist but docker would not say; not removing anything" ;;
      present)
        # The id recorded by *this* state directory, and nothing weaker. A label
        # only proves some checkout of this script started it, and that is not
        # the same claim: two checkouts share the default name, so checkout B
        # starting up would find A's running container carrying the shared label
        # and remove it. A name collision that is not this directory's own
        # previous container is reported instead.
        previous="$(started_container)" || true
        actual="$(docker inspect -f '{{.Id}}' "$CONTAINER" 2>/dev/null)" || actual=""
        if [ -n "$previous" ] && [ "$actual" = "$previous" ] \
          && [ "$(docker inspect -f '{{.State.Running}}' "$CONTAINER" 2>/dev/null)" != true ]; then
          docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
          rm -f "$STATE_DIR/container"
        else
          die "a container named $CONTAINER is in the way and is not this checkout's own stopped one; rename with KANON_SEQUENCER_CONTAINER, or stop it where it was started"
        fi
        ;;
    esac
    ensure_image
    assert_image_runnable
  else
    [ -f "$HOST_CONFIG" ] || die \
      "no debug sequencer config at $HOST_CONFIG; run: $0 fetch (or $0 build)"
    HOST_BIN="$(sequencer_bin)"
  fi

  bedrock_up

  reset_state_dir
  home="$STATE_DIR/home"
  mkdir -p "$home"

  # Where the state directory appears to the sequencer, which is the only thing
  # `--home` can be given.
  if [ "$RUNTIME" = container ]; then
    guest_home="$CONTAINER_HOME"
  else
    guest_home="$home"
  fi

  local help
  local -a extra
  if ! help="$(sequencer_help)"; then
    log "$help"
    die "the sequencer could not run; if this came from \`fetch\`, the image is for x86_64 linux"
  fi
  extra=()
  case "$help" in
    *--listen-address*) extra+=(--listen-address 127.0.0.1) ;;
    *) log "no --listen-address in this build; the RPC binds to its own default" ;;
  esac
  case "$help" in
    # `--home` overrides the config's own `home` so the config can be used
    # exactly as committed. Without it the config's value applies, which in the
    # debug config is `.` -- and since both runtimes start the sequencer *in*
    # the state directory (a `cd` on the host, a `WORKDIR` in the image), the
    # RocksDB state still lands where this script can delete it. That is why
    # both are load-bearing rather than tidiness.
    *--home*) extra+=(--home "$guest_home") ;;
    *) log "no --home in this build; the config's own home applies, relative to $guest_home" ;;
  esac

  # RISC0_DEV_MODE makes block production fast enough for tests to run at all;
  # the proofs it produces are not real ones, which is why testnet verification
  # is a separate deliverable and not something CI claims.
  if [ "$RUNTIME" = container ]; then
    start_container "${extra[@]}"
  else
    start_host "${extra[@]}"
  fi

  local waited=0
  refresh_log
  until grep -q "RPC server started" "$STATE_DIR/sequencer.log" 2>/dev/null; do
    if ! sequencer_running; then
      log "sequencer exited during startup; last lines:"
      refresh_log
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
    refresh_log
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

# Stops everything `start` started, and deletes the run state only if it did.
#
# That condition is the whole shape of this function, and it is here because the
# obvious version destroyed a running chain. Deleting the state directory is the
# one irreversible thing here -- it is the sequencer's RocksDB -- and every step
# that stops something is a `docker` call that can fail for reasons that have
# nothing to do with this script. Measured, on a shell with no docker socket:
# `docker inspect` failed, which is indistinguishable from "no such container",
# so the container was never stopped; `docker compose down -v` failed into its
# `|| true`; and the home directory was then removed under a sequencer that was
# still serving RPC, which kept running off the unlinked inodes and would have
# lost the chain at its next restart.
#
# So a stop that could not stop something leaves the state alone and says so.
# The previous version could not hit this: the sequencer was a host process
# killed by pid, so it was always genuinely dead before its home was removed.
cmd_stop() {
  local stopped_everything=1

  # Both halves need docker -- bedrock always, the sequencer in the container
  # runtime -- so a daemon that cannot be reached means nothing here ran.
  if ! docker info >/dev/null 2>&1; then
    log "docker is not reachable, so nothing it is running was stopped"
    stopped_everything=0
  fi

  # Attempted regardless of the selected runtime: a `stop` run with a different
  # `KANON_SEQUENCER_RUNTIME` than the `start` before it should still clean up,
  # and removing a container that does not exist costs nothing.
  #
  # The last log is copied out first, because `docker rm` takes docker's copy
  # with it and a sequencer that misbehaved is exactly when the log is wanted.
  local id
  id="$(started_container)" || true
  if [ "$stopped_everything" = 1 ] && [ -n "$id" ] \
    && [ "$(container_state "$id")" = present ]; then
    log "stopping sequencer container $id"
    # Only in the container runtime, and the distinction is not pedantic. The
    # removal above is deliberately attempted whatever the runtime, so a stray
    # container gets cleaned up either way -- but a *stray* one is by definition
    # not what produced the log this file holds. A developer whose container
    # died at startup and who switched to `KANON_SEQUENCER_RUNTIME=host` to get
    # a working sequencer would otherwise have `stop` overwrite the host run's
    # log with the dead container's, losing the diagnostics for the run they
    # just stopped.
    if [ "$RUNTIME" = container ]; then
      docker logs "$id" >"$STATE_DIR/sequencer.log" 2>&1 || true
    fi
    # SIGINT, not docker's default SIGTERM. The pinned sequencer installs one
    # handler, `tokio::signal::ctrl_c()`, which is SIGINT and nothing else -- so
    # a SIGTERM is ignored until docker gives up and sends SIGKILL ten seconds
    # later, and the clean shutdown the comment below claims never happens. The
    # image also carries `STOPSIGNAL SIGINT` so that a `docker stop` from
    # anywhere else gets it right; this flag is belt and braces for an image
    # predating that.
    docker stop --signal SIGINT "$id" >/dev/null 2>&1 || true
    docker rm -f "$id" >/dev/null 2>&1 || log "could not remove $id"

    # The id file is the only record that this container exists, so it is
    # dropped only once the container demonstrably does not. Removing it after a
    # failed `docker rm` left the *next* `stop` with nothing to look up: that
    # one would find no id, conclude nothing was running, and delete the
    # database of a container still holding it. The first stop refused
    # correctly and the second undid the refusal.
    case "$(container_state "$id")" in
      absent) rm -f "$STATE_DIR/container" ;;
      present) log "container $id is still there"; stopped_everything=0 ;;
      *) log "could not tell whether container $id is still there"; stopped_everything=0 ;;
    esac
  fi

  if [ -f "$STATE_DIR/pid" ]; then
    local pid
    pid="$(cat "$STATE_DIR/pid")"
    if kill -0 "$pid" 2>/dev/null; then
      log "stopping sequencer (pid $pid)"
      # SIGINT, because that is the only signal the sequencer listens for:
      # `listen_for_shutdown_signal` awaits `tokio::signal::ctrl_c()` and
      # installs no SIGTERM handler. This line sent SIGTERM for as long as the
      # script has existed, under a comment asserting it was the clean path, so
      # every `stop` until now has been waiting thirty seconds for a signal the
      # process was never going to act on and then killing it outright. A killed
      # sequencer can leave its RocksDB home locked, which is what the clean
      # path is for.
      kill -INT "$pid" 2>/dev/null || true
      local waited=0
      while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 30 ]; do
        waited=$((waited + 1))
        sleep 1
      done
      # `kill -9` returns as soon as the signal is queued, not when the process
      # is gone, and a sequencer in an uninterruptible wait can outlive it for a
      # while. So it is waited on too, and the pid file is removed only once the
      # process really has gone: it is the only record that there was one, and
      # deleting it while the process lives makes `sequencer_running` answer
      # "nothing here" and lets the RocksDB be unlinked underneath it. That is
      # the same failure this function was rewritten to prevent on the container
      # side, and leaving it open here would have kept it alive on the other.
      kill -9 "$pid" 2>/dev/null || true
      waited=0
      while kill -0 "$pid" 2>/dev/null && [ "$waited" -lt 30 ]; do
        waited=$((waited + 1))
        sleep 1
      done
    fi
    if kill -0 "$pid" 2>/dev/null; then
      log "sequencer $pid survived SIGKILL"
      stopped_everything=0
    else
      rm -f "$STATE_DIR/pid"
    fi
  fi

  local dir
  dir="$(checkout_dir)/bedrock"
  if [ "$stopped_everything" = 1 ] && [ -d "$dir" ]; then
    log "stopping bedrock"
    ( cd "$dir" && docker compose down -v ) \
      || { log "could not bring bedrock down"; stopped_everything=0; }
  fi

  # The last word, and it asks the question directly rather than trusting the
  # bookkeeping above: something still answering here is something whose database
  # must not be deleted.
  if sequencer_running; then
    log "a sequencer is still running"
    stopped_everything=0
  fi

  if [ "$stopped_everything" = 0 ]; then
    log "run state left at $STATE_DIR; deleting it under a running sequencer takes its database away"
    die "stop did not stop everything, so it deleted nothing"
  fi

  remove_state_subpath home
  log "stopped"
}

# The sequencer's log, from wherever it currently lives.
#
# A running container's log is docker's, and the file under `$STATE_DIR` is only
# as fresh as the last `refresh_log`; once the container is gone, `stop` has
# copied the final one into that file. So the container is preferred when it is
# there and the file is the fallback, which also covers the host runtime.
cmd_logs() {
  local lines="${2:-50}"
  # Gated on the runtime and not merely on a container existing. A container
  # left behind by an earlier run is still findable by name, and preferring it
  # would print a dead container's output while a host sequencer is live, with
  # nothing to say which one the reader is looking at.
  local id
  id="$(started_container)" || true
  if [ "$RUNTIME" = container ] && [ -n "$id" ] \
    && docker inspect "$id" >/dev/null 2>&1; then
    docker logs --tail "$lines" "$id" 2>&1
  else
    tail -n "$lines" "$STATE_DIR/sequencer.log"
  fi
}

usage() {
  cat >&2 <<'USAGE'
usage: scripts/lez-sequencer.sh <command>

  pin     print the LEZ revision Cargo.lock resolves, which is what gets built
  format  print the image format `start` requires, which the publish workflow stamps
  build   clone that revision and build sequencer_service (minutes, cached)
  fetch   pull a prebuilt sequencer for that revision instead of building it
  start   start bedrock and the sequencer; prints the RPC URL on stdout
  smoke   assert the running sequencer serves RPC and is producing blocks
  stop    stop both and delete the run state
  logs    tail the sequencer log

Both `build` and `fetch` leave a LEZ checkout behind, and `start` needs one
either way: bedrock's compose file is read from it on the host.

environment:
  KANON_LEZ_CACHE         checkout and build cache (default ~/.cache/kanon-lez)
  KANON_SEQUENCER_STATE   run state (default target/lez-sequencer)
  KANON_SEQUENCER_PORT    sequencer RPC port (default 3055)
  KANON_SEQUENCER_TIMEOUT readiness timeout in seconds (default 180)
  KANON_SEQUENCER_IMAGE   prebuilt image for `fetch` and for the container
                          (default ghcr.io/equilibriumco/kanon-lez-sequencer)
  KANON_SEQUENCER_RUNTIME container (default) runs the sequencer in a container
                          from that image, with --network host; host runs the
                          binary as a host process, which is what to use where
                          the image's glibc will not run (NixOS, non-x86_64,
                          anything but linux). See adr/0034.
  KANON_SEQUENCER_CONTAINER
                          container name (default kanon-lez-sequencer)
USAGE
  exit 64
}

case "${1:-}" in
  pin) cmd_pin ;;
  format) cmd_format ;;
  build) cmd_build ;;
  fetch) cmd_fetch ;;
  start) cmd_start ;;
  smoke) cmd_smoke ;;
  stop) cmd_stop ;;
  logs) cmd_logs "$@" ;;
  *) usage ;;
esac
