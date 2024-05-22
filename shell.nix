# Copyright Gary Guo.
#
# SPDX-License-Identifier: MIT OR Apache-2.0

{pkgs ? import <nixpkgs> {}}:
pkgs.mkShell {
  buildInputs = with pkgs; [sqlite];
  nativeBuildInputs = with pkgs; [rustup];
}
