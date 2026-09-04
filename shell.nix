# Host-side build environment for the Kanon repo.
#
# This covers the *host* toolchain only. Guest code cross-compiles to
# riscv32im-risc0-zkvm-elf using RISC Zero's own Rust toolchain, which is
# managed by `rzup` and lives under ~/.risc0, outside Nix, because rzup ships
# prebuilt binaries. See cost-baseline/README.md for the rzup setup, and note
# that NixOS needs `programs.nix-ld.enable = true` for those binaries to run.
#
# GPU proving is opt-in, because it pulls in unfree CUDA packages:
#
#   nix-shell --arg withCuda true
#
# then build and run with `--features cuda`.
{ withCuda ? false }:
let
  fenix = import (builtins.fetchTarball {
    url = "https://github.com/nix-community/fenix/archive/main.tar.gz";
  }) {};

  pkgs = import <nixpkgs> { config.allowUnfree = withCuda; };

  rustToolchain = fenix.stable.withComponents [
    "rustc"
    "cargo"
    "clippy"
    "rustfmt"
    "rust-src"
    "rust-analyzer"
  ];

  # risc0 compiles its CUDA kernels at build time, so the toolkit is required,
  # not just the driver.
  cudaInputs = (with pkgs.cudaPackages; [
    cuda_nvcc
    cuda_cudart
    cudatoolkit
  ]) ++ [
    # Enabling risc0's `cuda` feature pulls in the Groth16 prover, whose
    # dependency chain builds a protobuf crate.
    pkgs.protobuf
  ];

  # `find_cuda_helper`, which risc0's CUDA dependency chain builds against, only
  # recognises a CUDA install on Linux if the base directory contains `lib64/`
  # or `targets/x86_64-linux/`. Nix ships `lib/`, so detection fails outright and
  # the build panics with "Could not find a cuda installation". This shim presents
  # the layout it expects, without copying anything.
  cudaShim = pkgs.runCommand "cuda-fhs-shim" { } ''
    mkdir -p "$out"
    ln -s ${pkgs.cudaPackages.cudatoolkit}/lib     "$out/lib64"
    ln -s ${pkgs.cudaPackages.cudatoolkit}/include "$out/include"
    ln -s ${pkgs.cudaPackages.cudatoolkit}/bin     "$out/bin"
  '';
in
  pkgs.mkShell {
    buildInputs = [
      rustToolchain
      pkgs.cargo-deny # the M1 licence gate: fails CI on BUSL or copyleft deps
      pkgs.cargo-nextest
      pkgs.git
      pkgs.llvmPackages.libclang
      pkgs.openssl
      pkgs.openssl.dev
      pkgs.pkg-config
      pkgs.zlib
    ] ++ pkgs.lib.optionals withCuda cudaInputs;

    shellHook = ''
      # rzup installs cargo-risczero, r0vm, and the guest Rust toolchain here.
      export PATH="$HOME/.cargo/bin:$PATH"

      export LIBCLANG_PATH=${pkgs.llvmPackages.libclang.lib}/lib
      export LD_LIBRARY_PATH=${pkgs.llvmPackages.libclang.lib}/lib:${pkgs.zlib}/lib:${pkgs.stdenv.cc.cc.lib}/lib:${pkgs.openssl.out}/lib
      export RUST_SRC_PATH=${rustToolchain}/lib/rustlib/src/rust/library

      # bindgen invokes libclang, which on NixOS has no default include paths, so
      # even <stddef.h> is unresolvable without being told where to look.
      export BINDGEN_EXTRA_CLANG_ARGS="\
        -I${pkgs.glibc.dev}/include \
        -I${pkgs.llvmPackages.libclang.lib}/lib/clang/${pkgs.lib.versions.major pkgs.llvmPackages.libclang.version}/include"
    '' + pkgs.lib.optionalString withCuda ''
      export CUDA_PATH=${cudaShim}
      export CUDA_ROOT="$CUDA_PATH"
      export CUDA_TOOLKIT_ROOT_DIR="$CUDA_PATH"
      # Read by find_cuda_helper's read_env, and the only variable its Linux path
      # actually consults.
      export CUDA_LIBRARY_PATH="$CUDA_PATH"
      # The real libcuda comes from the running driver, not from Nix. The stub in
      # the toolkit satisfies link time; this satisfies run time.
      export LD_LIBRARY_PATH="/run/opengl-driver/lib:$LD_LIBRARY_PATH"

      export PROTOC="${pkgs.protobuf}/bin/protoc"

      # sppark 0.1.15 calls assert() inside __global__ functions, which nvcc 12.8
      # rejects outright ("calling a __host__ function from a __global__
      # function"). Compiling asserts out sidesteps it.
      export CXXFLAGS="-DNDEBUG ''${CXXFLAGS:-}"
      export CFLAGS="-DNDEBUG ''${CFLAGS:-}"

      echo "CUDA enabled: $(nvcc --version | tail -1)"
    '';
  }
