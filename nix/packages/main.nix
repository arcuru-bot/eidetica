# Core eidetica package (binary)
{
  craneLib,
  releaseArgs,
  debugArgs,
}: let
  eidetica-bin = craneLib.buildPackage (releaseArgs
    // {
      pname = "eidetica-bin";
      # No --all-features: eidetica-bin declares no features of its own, and the
      # flag would enable every feature of any it later gains — including a
      # passthrough to the library's test-only `testing` hooks. Default features
      # already resolve to the library's `full`. Enforced by the
      # `release-features` lint.
      cargoExtraArgs = "-p eidetica-bin";
      doCheck = false; # Tests run separately with nextest
      meta = {
        description = "Eidetica binary";
        mainProgram = "eidetica";
      };
    });

  # Debug build for CI checks (fast: reuses cargoArtifactsDebug)
  eidetica-bin-debug = craneLib.buildPackage (debugArgs
    // {
      pname = "eidetica-bin-debug";
      cargoExtraArgs = "-p eidetica-bin";
      doCheck = false;
    });
in {
  inherit eidetica-bin eidetica-bin-debug;
}
