# Benchmark packages
{
  craneLib,
  benchArgs,
  eidLib,
}: let
  defaultBenches = "--bench benchmarks --bench backend_benchmarks --bench table_cache_benchmarks --bench scale_benchmarks";

  # Build bench artifacts (cached in Nix store)
  # Uses cargo bench --no-run to compile without executing
  bench-artifacts = craneLib.mkCargoDerivation (benchArgs
    // {
      pname = "bench-artifacts";
      buildPhaseCargoCommand = "cargo bench --no-run --workspace --all-features";
      doInstallCargoArtifacts = true;
    });

  # Default benchmark derivation (hermetic) — excludes extended benchmarks
  bench = craneLib.mkCargoDerivation (benchArgs
    // {
      pname = "bench";
      cargoArtifacts = bench-artifacts;
      buildPhaseCargoCommand = "cargo bench ${defaultBenches} --all-features";
      doCheck = false;
      meta = {
        description = "Eidetica benchmark execution (default)";
      };
    });

  # All benchmarks (hermetic) — includes extended benchmarks
  bench-all = craneLib.mkCargoDerivation (benchArgs
    // {
      pname = "bench-all";
      cargoArtifacts = bench-artifacts;
      buildPhaseCargoCommand = "cargo bench --workspace --all-features";
      doCheck = false;
      meta = {
        description = "Eidetica benchmark execution (all, including extended)";
      };
    });

  # Interactive benchmark runners
  bench-runner = eidLib.mkCargoRunner {
    name = "bench-runner";
    command = "cargo bench ${defaultBenches} --all-features";
  };

  bench-all-runner = eidLib.mkCargoRunner {
    name = "bench-all-runner";
    command = "cargo bench --workspace --all-features";
  };
in {
  builds = {
    default = bench;
    all = bench-all;
  };

  runners = {
    default = bench-runner;
    all = bench-all-runner;
  };

  artifacts = bench-artifacts;
}
