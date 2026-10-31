{ pkgs ? import <nixpkgs> {} }:

let
  linuxDeps = with pkgs; [
    pkg-config
    alsa-lib
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
    xorg.libxcb
    libGL
    vulkan-loader
  ];

  darwinDeps = with pkgs.darwin.apple_sdk.frameworks; [
    AppKit
    CoreAudio
    AudioToolbox
    CoreFoundation
    Metal
    Security
  ];
in
pkgs.mkShell {
  name = "scout-dev";

  nativeBuildInputs = with pkgs; [
    pkg-config
  ];

  buildInputs = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    rust-analyzer
    sqlite
  ] ++ (if pkgs.stdenv.isLinux then linuxDeps else darwinDeps);

  shellHook = if pkgs.stdenv.isLinux then ''
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath linuxDeps}:$LD_LIBRARY_PATH"
  '' else '''';
}
