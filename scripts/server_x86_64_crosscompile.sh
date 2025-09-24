#!/bin/sh
# Prerequisites:
# rustup target add x86_64-unknown-linux-gnu && brew tap SergioBenitez/osxct && brew install x86_64-unknown-linux-gnu
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-unknown-linux-gnu-gcc cargo build --release -p prismata_server --target x86_64-unknown-linux-gnu