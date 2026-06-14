set -euo pipefail

apt-get update
apt-get install -y --no-install-recommends ca-certificates curl git build-essential pkg-config python3 python3-pip ninja-build cmake
python3 -m pip install --break-system-packages --no-cache-dir cmake==3.30.0
export PATH="$HOME/.local/bin:$PATH"

if [ ! -x /usr/bin/rustup ]; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
fi
source "$HOME/.cargo/env"
rustup default stable

rm -rf /repo/core/build-linux-arm64
cd /repo
cmake -S core -B core/build-linux-arm64 -G Ninja -DCMAKE_BUILD_TYPE=Debug -DCMAKE_EXE_LINKER_FLAGS="-static-libstdc++"
cmake --build core/build-linux-arm64 --target prisma_core_tests -j 2

export PRISMA_CORE_LIB_DIR=/repo/core/build-linux-arm64
export PRISMA_CPP_HEADERS_DIR=/repo/core/include

cd /repo/shell
cargo test --manifest-path /repo/shell/Cargo.toml -p prisma-runtime --test smoke_differential -- --exact "Dispatcher: CMP + JE branches to the taken leg on equal operands"