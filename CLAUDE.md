# Prismata

Voxel-based research prototype built with Rust and Bevy. The workspace contains:
- `client` - Bevy-based voxel client with egui UI
- `server` - Game server
- `server_lib` - Shared server library
- `protocol` - Network protocol definitions

## Development Commands

Use `cargo clippy` to check for lints and `cargo fmt` to format code.

**Never use `cargo build` or `cargo run` directly.** These commands require specific environment setup.

## Web Build

To build the client for WebGPU/WASM:

```sh
./scripts/client_web_build.sh
```

This builds the client targeting wasm32-unknown-unknown with WebGPU support and outputs to `./build/`.

## NixOS

On NixOS, run commands through nix-shell:

```sh
nix-shell --run "cargo clippy"
nix-shell --run "cargo fmt"
```
