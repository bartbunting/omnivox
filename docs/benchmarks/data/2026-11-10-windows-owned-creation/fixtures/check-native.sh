#!/bin/sh
set -eu
docker run --rm --platform linux/amd64 --user 1000:1000 \
 --env HOME=/workspace/omnivox/target/emacsvox-home \
 --env CARGO_HOME=/workspace/omnivox/target/emacsvox-cargo-home \
 --env CARGO_TARGET_DIR=/workspace/omnivox/target/guard-windows-check \
 --env CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-win32 \
 --env CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-win32 \
 --env AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
 --env CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-win32 \
 --volume /home/bart/src/omnivox-lightweight-reload:/workspace/omnivox \
 --volume /home/bart/src/omnivox/.git:/home/bart/src/omnivox/.git:ro \
 --workdir /workspace/omnivox emacsvox-omnivox-windows-gnu:lightweight-20261110 \
 sh -eu -c 'cargo clippy --locked -p omnivox-cli -p omnivox-tts --all-targets --target x86_64-pc-windows-gnu -- -D warnings; cargo test --locked -p omnivox-cli -p omnivox-tts --target x86_64-pc-windows-gnu --no-run'
