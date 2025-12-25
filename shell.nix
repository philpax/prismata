{ pkgs ? import <nixpkgs> { } }:

with pkgs;

mkShell rec {
  nativeBuildInputs = [
    pkg-config
    wasm-bindgen-cli
    binaryen # for wasm-opt
  ];
  buildInputs = [
    udev alsa-lib-with-plugins vulkan-loader
    xorg.libX11 xorg.libXcursor xorg.libXi xorg.libXrandr # To use the x11 feature
    libxkbcommon wayland wayland.dev wayland-protocols # To use the wayland feature
  ];
  LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
  PKG_CONFIG_PATH = lib.makeSearchPath "lib/pkgconfig" buildInputs
    + ":${wayland.dev}/lib/pkgconfig";
}
