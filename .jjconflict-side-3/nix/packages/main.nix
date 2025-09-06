# Core eidetica packages (library, binary, and xtask)
{
  craneLib,
  releaseArgs,
  baseArgs,
  pkgs,
}: let
  # Library crate build
  # Uses workspace-wide release artifacts, package-specific flags added here
  eidetica-lib = craneLib.buildPackage (releaseArgs
    // {
      pname = "eidetica";
      cargoExtraArgs = "-p eidetica --all-features";
      doCheck = false; # Tests run separately with nextest
      meta = {
        description = "Eidetica library - A P2P decentralized database";
      };
    });

  # Binary crate build
  # Uses workspace-wide release artifacts, package-specific flags added here
  eidetica-bin = craneLib.buildPackage (releaseArgs
    // {
      pname = "eidetica-bin";
      cargoExtraArgs = "-p eidetica-bin --all-features";
      doCheck = false; # Tests run separately with nextest
      meta = {
        description = "Eidetica binary";
        mainProgram = "eidetica";
      };
    });

  # Main package alias
  eidetica = eidetica-bin;

  # Build xtask with shell completions
  xtask = craneLib.buildPackage (baseArgs
    // {
      pname = "xtask";
      cargoExtraArgs = "--package xtask";
      doCheck = false;

      nativeBuildInputs = baseArgs.nativeBuildInputs ++ [pkgs.installShellFiles];

      postInstall = ''
        installShellCompletion --cmd xtask \
          --bash <($out/bin/xtask completions bash) \
          --zsh <($out/bin/xtask completions zsh) \
          --fish <($out/bin/xtask completions fish)
      '';

      meta.mainProgram = "xtask";
    });
in {
  inherit eidetica eidetica-lib eidetica-bin xtask;
}
