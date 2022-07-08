#
# Configures a Rust and Cargo dev environment with the specific
# tools needed for working on harrison.ai Rust projects.
#
FROM rust:1.60-slim

# Install extra system dependencies not included in the slim base image.
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    # For helping to build some Rust crates.
    libssl-dev \
    pkg-config \
    # For their general extreme usefulness.
    jq \
    curl \
    # For cross-compilation to AWS Graviton2.
#     gcc-aarch64-linux-gnu \
    libc-dev-arm64-cross \
    musl-tools \
    make \
  && apt-get clean \
  && rm -rf /var/lib/apt/lists/*

# General dev tools.
RUN rustup component add rustfmt
RUN rustup component add clippy

# Install some helpful extra Rust utilities.
# We build these in a single `RUN` invocation with a shared target-dir
# so that cargo can share intermediate compile artifacts rather than
# having to build everything from scratch, which saves a lot of compile
# time in release mode.
RUN export CARGO_TARGET_DIR=/tmp/cargo-install-target \
  # cargo-deny: used for dependency license and security checks.
  # Disabling default features lets it use the system ssl library,
  # which should reduce overall size of the docker image.
  && cargo install --version="0.11.4" --no-default-features cargo-deny \
  # cargo-about: used for generating license files for distribution to consumers,
  #              which may be required for compliance with some open-source licenses.
  && cargo install --version="0.5.1" cargo-about \
  # cargo-release: used for cutting releases.
  && cargo install --version="0.20.5" cargo-release \
  # Remove temporary files from the final image.
  && rm -rf "$CARGO_HOME/registry" /tmp/cargo-install-target

# Configure cross-compilation support for AWS Graviton2 processors,
# and x86_64 linux static binary.
# We use musl to help produce smaller docker images.
RUN rustup target add aarch64-unknown-linux-musl x86_64-unknown-linux-musl

# Extra stuff required for x86_64 musl
# openssl crate supports OpenSSL 1.0.1 to 1.1.1 (not 3.0.0)
ENV SSL_VER="1.1.1l" \
   ZLIB_VER="1.2.12" \
   CC="musl-gcc -static -idirafter /usr/include/ -idirafter /usr/include/aarch64-linux-gnu/"\
   PREFIX=/musl \
   PATH=/usr/local/bin:/root/.cargo/bin:$PATH \
   PKG_CONFIG_PATH=/usr/local/lib/pkgconfig \
   LD_LIBRARY_PATH=$PREFIX

# Build zlib used in openssl
RUN curl -sSL https://zlib.net/zlib-$ZLIB_VER.tar.gz > zlib-${ZLIB_VER}.tar.gz && \
   echo "91844808532e5ce316b3c010929493c0244f3d37593afd6de04f71821d5136d9" \
   zlib-${ZLIB_VER}.tar.gz | sha256sum --check && \
   tar -xzf zlib-${ZLIB_VER}.tar.gz && \
   rm zlib-${ZLIB_VER}.tar.gz && \
   cd zlib-$ZLIB_VER && \
   CC="musl-gcc -fPIC -pie" LDFLAGS="-L$PREFIX/lib" CFLAGS="-I$PREFIX/include" ./configure --static --prefix=$PREFIX && \
   make -j$(nproc) && make install && \
   cd .. && rm -rf zlib-$ZLIB_VER

# Build openssl
RUN curl -sSL https://www.openssl.org/source/openssl-${SSL_VER}.tar.gz > openssl-${SSL_VER}.tar.gz && \
   echo "0b7a3e5e59c34827fe0c3a74b7ec8baef302b98fa80088d7f9153aa16fa76bd1" \
   openssl-${SSL_VER}.tar.gz | sha256sum --check && \
   tar -xzf openssl-${SSL_VER}.tar.gz && \
   rm openssl-${SSL_VER}.tar.gz && \
   cd openssl-$SSL_VER && \
   ./Configure no-shared -DOPENSSL_NO_SECURE_MEMORY -fPIC --prefix=$PREFIX --openssldir=$PREFIX/ssl linux-aarch64 && \
   env C_INCLUDE_PATH=$PREFIX/include make depend 2> /dev/null && \
   make -j$(nproc) && make install_sw && \
   cd .. && rm -rf openssl-$SSL_VER

# # for the cc crate
# ENV HOST_CC=gcc

ENV PATH=$PREFIX/bin:$PATH \
   PKG_CONFIG_ALLOW_CROSS=true \
   PKG_CONFIG_ALL_STATIC=true \
   PKG_CONFIG_PATH=$PREFIX/lib/pkgconfig \
   OPENSSL_STATIC=true \
   OPENSSL_DIR=$PREFIX \
   SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt \
   SSL_CERT_DIR=/etc/ssl/certs \
   LIBZ_SYS_STATIC=1

# An easy way to run our standard suite of CI checks.
COPY ./scripts/cargo-hai-all-checks /usr/local/cargo/bin
