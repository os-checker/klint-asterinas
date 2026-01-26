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
              };

              buildInputs = with pkgs; [ sqlite ];
              doCheck = false;

              # If kernel rustdoc tests are enabled, user would need a matching version of rustdoc.
              postInstall = ''
                ln -s "${lib.getExe' rustc "rustdoc"}" $out/bin/klint-rustdoc
              '';

              passthru.rustc = rustc;
            };

        formatter = pkgs.nixfmt-tree;
      }
    );
}
