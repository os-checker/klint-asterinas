{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };
      in
      {
        devShells.rustup = pkgs.mkShell {
          buildInputs = with pkgs; [ sqlite ];
          nativeBuildInputs = with pkgs; [ rustup ];
        };

        formatter = pkgs.nixfmt-tree;
      }
    );
}
