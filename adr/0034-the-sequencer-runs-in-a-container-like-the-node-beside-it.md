# 34. The sequencer runs in a container, like the node beside it

- **Status**: accepted; supersedes part of 12
- **Milestone**: M2, untracked — there is no task id for it in the internal gantt
- **Requirements**: S2
- **Artefacts**: `scripts/lez-sequencer.sh`, `.github/workflows/lez-sequencer-image.yml`

## Context

ADR 12 built the standalone sequencer out of two halves that are not alike. Bedrock
runs as a container, from the image LEZ's own `bedrock/docker-compose.yml` names. The
sequencer runs as a **host process**, from a binary copied out of an image that is
deliberately not runnable — "a transport format for build output, not a runnable
service image".

That asymmetry has a cost, and it is the binary's linkage. `ldd` on the published
`sequencer_service` names `libssl`, `libcrypto`, `libstdc++`, `libgcc_s`, `libz`,
`libbrotlienc`, `libbrotlidec`, `libbrotlicommon`, `libzstd`, `libm` and `libc`, plus
the glibc loader. The image is built on `ubuntu-24.04`, so `fetch` produces something
that runs on that ABI and on hosts close enough to it. Everywhere else — NixOS, a
non-x86_64 machine, anything that is not linux — the only route in is `build`, which is
an hours-long compile of third-party code that ADR 12 already established does not fit a
runner. The script says as much in its own error message: *if this came from `fetch`,
the image is for x86_64 linux*.

So the harness a developer runs and the harness CI runs differ in a way nothing checks,
and the difference is exactly the fragile part.

## Decision

**The published image becomes a service image, and keeps being a transport format.**
The base is `ubuntu:24.04` instead of `scratch`, with the runtime libraries `ldd` names
installed, an `ENTRYPOINT` on the binary, and a `WORKDIR` at the directory the run state
is mounted into. The payload still lands at the same `/lez/...` paths, so `fetch` is
unchanged and one copy of the binary serves both uses.

**`start` runs the sequencer from that image, and that is the default.**
`KANON_SEQUENCER_RUNTIME=host` keeps the previous path for the hosts the image cannot
serve, which is the case `build` exists for. The two paths share the readiness loop, the
feature detection and the state directory; they differ in how the process is started and
where its log is read from.

**`--network host`, so the committed config is used exactly as it ships.**
`bedrock_config.node_url` in LEZ's debug config is `http://localhost:18080`, and in a
bridge network `localhost` is the container itself. Host networking is what lets the
config stay byte for byte the file in the checkout, which is the property ADR 12 chose
when it declined to make the URL a knob. It also puts the RPC straight onto the host's
port, so nothing downstream — `smoke`, the end-to-end tests, `ci.yml` — has to know a
container is involved.

**The container runs as the invoking user.** RocksDB writes into the bind-mounted state
directory, which lives under `target/`. A root-owned state directory is one `stop`
cannot delete and one `reset_state_dir` would refuse on the next run.

**The transition needs nobody to remember a flag.** Every image published before this
decision is `FROM scratch` and carries no entrypoint, and the publish workflow's
"already published?" check would have reported one as current. That check now pulls the
image and inspects its entrypoint, so an old-shaped one is rebuilt on the next run;
`start` makes the same check and refuses with a message naming the workflow rather than
letting `docker run` fail as though the script were broken.

## Consequences

- **Host networking is free on linux and a setting elsewhere.** It is not linux-only:
  Docker Desktop has supported `--network host` on macOS and Windows since 4.34, and
  documents it as working in both directions, so a port a container binds is reachable
  from the host's own `localhost`. But it is off by default and has to be turned on in
  *Settings → Resources → Network*, it is layer 4 only, and it is incompatible with
  Enhanced Container Isolation. Neither target here is affected — CI is `ubuntu-24.04`
  and the machines this is developed on are linux — so the macOS path is **untested**
  rather than unsupported, and this is written down so nobody reads a green CI run as
  evidence for it. On a host where the image's glibc will not run at all,
  `KANON_SEQUENCER_RUNTIME=host` with `build` is the route, as it was before.
- **A LEZ checkout is still required to start anything.** Bedrock is still LEZ's own
  compose project, and `docker compose` reads that file from the host. So `fetch` (or
  `build`) still runs first, in both runtimes, and `bedrock_up` now says so when the
  directory is missing.
- **One function decides whether anything might still be running, and every caller asks
  it.** Deleting the run state is the one irreversible act here — it is the sequencer's
  RocksDB — and three rounds of review found *seven* ways to reach "nothing is running"
  by mistake, in five different callers: a daemon that could not be reached, a `docker
  rm` that failed, an id file dropped anyway, a label that proved the wrong thing, an
  inspect error read as an absence, twice. Every one was a caller asking its own version
  of the question and getting a fail-open answer from a command that had not said no.
  The fix that stuck was not another check but a single `sequencer_running`, phrased so
  that **uncertainty answers yes**, used by the readiness loop, by `start`'s refusal and
  by the gate on deletion alike. `stop` now attempts, then asks once; the attempts report
  to the reader and decide nothing.
- **The sequencer only listens for SIGINT, so both runtimes send it.**
  `listen_for_shutdown_signal` awaits `tokio::signal::ctrl_c()` and installs no
  SIGTERM handler, so docker's default stop signal is ignored until the SIGKILL that
  follows it ten seconds later. The image carries `STOPSIGNAL SIGINT` and `stop` passes
  `--signal SIGINT` as well; the host path sent SIGTERM under a comment claiming it was
  the clean path, which means every `stop` since the script was written has waited out
  its timeout and then killed the process. A sequencer killed rather than shut down can
  leave its RocksDB home locked.
- **The image carries a format label, because the tag cannot answer the question.**
  The tag is the LEZ revision, so two images built from one revision by different
  Dockerfiles are indistinguishable by it — and `ensure_image` uses a local image as-is,
  so a developer holding the older one never pulls the replacement. The entrypoint check
  only ever catches the single transition it was written for. `scripts/lez-sequencer.sh
  format` owns the number, the workflow stamps what the script asks for rather than
  keeping a second copy, and both the publish decision and `start` compare against it.
- **A container is addressed by the id `start` recorded, not by its name.**
  `KANON_SEQUENCER_CONTAINER` is a caller's variable: a `start` under a custom name
  followed by a plain `stop` would look up the default, find nothing, conclude nothing
  was running, and delete the state directory out from under the real container. The id
  goes in the state directory and every later operation uses it; the name is used only
  where docker requires one, and a name collision with a container this script did not
  label is refused rather than removed.
- **The image is larger**, an ubuntu base plus the payload rather than the payload
  alone. It is pulled once per LEZ revision and cached; against the hour of build time
  it absorbs, this is not a trade worth tuning.
- **There are two start paths and CI exercises one.** The `sequencer` job runs the
  container path, which is the default and the one most people get. The host path is
  covered only by being the one that was there before, and by a developer on NixOS
  using it. That is a real gap and it is recorded rather than closed: a second CI job
  running the same suite the other way would double the job's cost to re-assert a path
  this repository did not change.
- **No image of the new shape exists until the publish workflow next runs.** Until it
  does, `start` in the default runtime refuses and names the workflow. That refusal is
  the intended behaviour of the transition, not a failure of it.

## Alternatives considered

- **Put the sequencer on Bedrock's compose network.** A compose override adding it as a
  second service, with `network_mode: "service:<node>"` so that `localhost:18080` is
  Bedrock, would keep bridge networking and need no host-networking setting anywhere.
  Rejected for now on two counts: the port has to be published by whichever container
  owns the namespace, which means editing LEZ's own service to expose `3055`, and the
  arrangement couples the script to LEZ's compose service name. If host networking
  turns out to be a nuisance on a machine somebody actually uses, this is the shape to
  reach for.
- **Rewrite `bedrock_config.node_url` and run on a bridge network.** Rejected for ADR
  12's reason: the value of pointing the sequencer at the config that ships in the
  checkout is that it is the config that ships in the checkout.
- **Replace the host path rather than keep both.** Rejected. It is the only path on a
  host whose libc the image does not match, and NixOS is one of the machines this is
  developed on.
- **Build the sequencer statically, or against musl.** Rejected: it is third-party code
  with its own `rust-toolchain.toml`, and changing how it links is a larger claim about
  someone else's build than this harness has any business making.
