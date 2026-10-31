{
  description = "Scout — Fast, local sample library manager";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Linux native build and runtime dependencies (audio + windowing + graphics)
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

        # macOS native frameworks
        darwinDeps = with pkgs.darwin.apple_sdk.frameworks; [
          AppKit
          CoreAudio
          AudioToolbox
          CoreFoundation
          Metal
          Security
        ];
      in
      {
        devShells.default = pkgs.mkShell {
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
        };
      }
    );
}
