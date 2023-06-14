#!/usr/bin/env bash
set -euo pipefail

ARCH=$(uname -m)

# only for x86_64 now.
if [ "X${ARCH}" != "Xx86_64" ]; then
        echo "Unsupported platform: ${ARCH}"
        exit 1
fi

export TARGET=x86_64-apple-darwin
MACOS_SDK_VER=MacOSX10.10
CTARGET=${TARGET}14
OSXCROSSPATH=$(pwd)/osxcross/target/bin
CMAKEPATH=$(pwd)/cmake-3.23.1-linux-x86_64/bin

export PATH="${HOME}/.cargo/bin":${CMAKEPATH}:${OSXCROSSPATH}:${PATH}

# tikv-jemalloc-sys
for p in cc c++ ranlib nm ar
do
    ln -fs ${OSXCROSSPATH}/${CTARGET}-${p} ${OSXCROSSPATH}/${TARGET}-${p}
done

# ring
export TARGET_CC=${OSXCROSSPATH}/${CTARGET}-clang
export TARGET_AR=${OSXCROSSPATH}/${CTARGET}-ar
export TARGET_CXX=${OSXCROSSPATH}/${CTARGET}-clang++-libc++

# cloudflare-zlib-sys, jemallocator
export TARGET_LDFLAGS="-fuse-ld=${OSXCROSSPATH}/${CTARGET}-ld"
export TARGET_CFLAGS="-msse4.2 ${TARGET_LDFLAGS}"

# boring-sys
export CXXFLAGS="${TARGET_LDFLAGS}"

# bindgen
export CMAKE=${OSXCROSSPATH}/${CTARGET}-cmake
export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=${OSXCROSSPATH}/../SDK/${MACOS_SDK_VER}.sdk/ -I${OSXCROSSPATH}/../SDK/${MACOS_SDK_VER}.sdk/usr/include/ --target=${CTARGET}"

# build lib and tests
# Use platform-common-default, cross compilation confuses the build script and it attempts to build
# seccomp bindgen otherwise.
# Unfortunately, cargo test doesn't respect --no-default-features (https://github.com/rust-lang/cargo/issues/7160)
# and we need to resort to using cargo build before open sourcing 
RUSTFLAGS="-D warnings" cargo build --release --no-default-features --features platform-common-default --target ${TARGET} $*