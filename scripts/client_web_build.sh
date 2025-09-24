#!/bin/sh
set -e
cargo build --release -p prismata_client --target wasm32-unknown-unknown --no-default-features --features webgpu
wasm-bindgen \
    --no-typescript \
    --target web  \
    --out-dir ./build/ \
    --out-name "prismata_client" \
    ./target/wasm32-unknown-unknown/release/prismata_client.wasm
wasm-opt -Oz -o build/prismata_client_bg.wasm build/prismata_client_bg.wasm

cat <<EOF > build/index.html
<!DOCTYPE html>
<html lang="en">
  <head>
    <title>Prismata</title>
  </head>
  <body style="margin: 0px; width: 100vw; height: 100vh;">
    <script type="module">
      import init from "./prismata_client.js";

      init().catch((error) => {
        if (
          !error.message.startsWith(
            "Using exceptions for control flow, don't mind me. This isn't actually an error!"
          )
        ) {
          throw error;
        }
      });
    </script>
  </body>
</html>
EOF
