# External tool: ifttt-lint — IfChange/ThenChange cross-file drift linter.
#
# Not in nixpkgs, so we package it ourselves via crane. This is a self-contained
# build: it does NOT reuse eidetica's `src`/`cargoArtifacts` (those are scoped to
# the workspace). crane reads ifttt-lint's own Cargo.lock to vendor its deps, so
# no manual cargoHash is needed — only the source `hash` below.
#
# Bump `rev` + `hash` together when upgrading. The structural Nix check
# (`lint.ifttt-structural`) and the impure CI diff job both consume this package.
{
  craneLib,
  pkgs,
  lib,
}: let
  src = pkgs.fetchFromGitHub {
    owner = "simonepri";
    repo = "ifttt-lint";
    rev = "v0.10.6";
    hash = "sha256-yx3GvQshf2L8QU5HurRQVFTrJ+ei7wCeVXxRt3EnM6E=";
  };

  commonArgs = {
    inherit src;
    pname = "ifttt-lint";
    version = "0.10.6";
    strictDeps = true;
    # Pure-Rust CLI; no system libraries to link.
    doCheck = false;
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;

  ifttt-lint = craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;
      meta = {
        description = "Stop cross-file drift with IfChange/ThenChange comments";
        homepage = "https://github.com/simonepri/ifttt-lint";
        license = lib.licenses.mit;
        mainProgram = "ifttt-lint";
      };
    });
in {
  inherit ifttt-lint;
}
