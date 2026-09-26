#!/bin/bash

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${CRATE_DIR}/../.." && pwd)"
WAMR_DIR=${ROOT}/wasm-micro-runtime
set -ex

# cd ${ROOT}
#
# cd ${WAMR_DIR}/wamr-compiler
# bash build_llvm.sh
# mkdir -p build
# cd build
# cmake .. -DWAMR_BUILD_PLATFORM=darwin
# make

cd ${ROOT}

echo "Building WAMR vmlib with CMake..."

# Create build directory if it doesn't exist
mkdir -p build
cd build

# Configure and build with CMake to get the vmlib
cmake ${CRATE_DIR} -DCMAKE_BUILD_TYPE=Release
make vmlib
 
