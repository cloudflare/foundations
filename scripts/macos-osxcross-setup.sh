#!/usr/bin/env bash
set -euo pipefail

ARCH=$(uname -m)

# only for x86_64 now.
if [ "X${ARCH}" != "Xx86_64" ]; then
        echo "Unsupported platform: ${ARCH}"
        exit 1
fi

##
## install rust x86_64-apple-darwin
##
curl https://sh.rustup.rs -sSf | sh -s -- -y --no-modify-path -t x86_64-apple-darwin

##
## configure rust x86_64-apple-darwin cross compile
##
cat << EOF >> .cargo/config

[target.x86_64-apple-darwin]
linker = "x86_64-apple-darwin14-clang"
ar = "x86_64-apple-darwin14-ar"
EOF

##
## install osxcross
##

# install latest cmake (osxcross need cmake >= 3.21)
CMAKEFILE=cmake-3.23.1-linux-x86_64.tar.gz
CMAKEURL=https://github.com/Kitware/CMake/releases/download/v3.23.1/${CMAKEFILE}
curl -LO ${CMAKEURL} && tar xzf ${CMAKEFILE}

# cleanup existing osxcross directory.
rm -fr osxcross

# macOS SDK URL.
MACOS_SDK_VER=MacOSX10.10
MACOS_SDKFILE=${MACOS_SDK_VER}.sdk.tar.xz
MACOS_SDKURL=https://s3.dockerproject.org/darwin/v2/${MACOS_SDKFILE}

# clone and build osxcross
git clone https://github.com/tpoechtrager/osxcross osxcross

cd osxcross
curl -O ${MACOS_SDKURL}
mv ${MACOS_SDKFILE} tarballs/
UNATTENDED=yes OSX_VERSION_MIN=10.7 ./build.sh

# patch cmake toolchain file to include asm.
cat << EOF >> target/toolchain.cmake

set(CMAKE_ASM_COMPILER "\${OSXCROSS_TARGET_DIR}/bin/\${OSXCROSS_HOST}-clang")

EOF