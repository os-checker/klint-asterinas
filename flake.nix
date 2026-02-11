{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        inherit (nixpkgs) lib;
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };

        rustc = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain;
      in
      {
        devShells.rustup = pkgs.mkShell {
          buildInputs = with pkgs; [ sqlite ];
          nativeBuildInputs = with pkgs; [ rustup ];
        };

        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [ sqlite ];
          nativeBuildInputs = [ rustc ];
        };

        packages.default =
          (pkgs.rustPlatform.buildRustPackage.override {
            inherit rustc;
            cargo = rustc;
          })
            {
              pname = "klint";
              version = "0.1.0";

              src = lib.fileset.toSource {
                root = ./.;
                fileset = lib.fileset.unions [
                  ./Cargo.toml
                  ./Cargo.lock
                  ./build.rs
                  ./.cargo
                  ./src
                ];
              };
              cargoLock = {
                lockFile = ./Cargo.lock;
                outputHashes = {
                  "compiletest_rs-0.11.2" = "sha256-kjdqn9MggFypzB6SVWAsNqD21wZYiv+dtPvyGNi/Wqo=";
                };
              };

              buildInputs = with pkgs; [ sqlite ];
              doCheck = false;

              # If kernel rustdoc tests are enabled, user would need a matching version of rustdoc.
              # klint provides a klint-rustdoc binary to ease the process. However, for nix, we already
              # know the path to the rustdoc binary, so just symlink and replace the wrapper.
              postInstall = ''
                ln -sf "${lib.getExe' rustc "rustdoc"}" $out/bin/klint-rustdoc
              '';

              passthru.rustc = rustc;
            };

        formatter = pkgs.nixfmt-tree;
      }
    );
}
