FROM rust:1.86-slim-bullseye AS build

RUN DEBIAN_FRONTEND=noninteractive apt-get update && apt-get install -y --no-install-recommends \
    apt-utils \
    software-properties-common \
    cmake \
    build-essential \
    wget \
    libclang-dev \
    libudev-dev \
    libssl-dev \
    pkg-config \
    libsasl2-dev \
    ca-certificates \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

RUN USER=root cargo new --bin solana
WORKDIR /solana
COPY . /solana

RUN cargo build --release


FROM rust:1.86-slim-bullseye

RUN DEBIAN_FRONTEND=noninteractive apt-get update && apt-get install -y --no-install-recommends \
    libssl1.1 \
    libsasl2-2 \
    libsasl2-modules \
    ca-certificates \
    wget \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin

COPY --from=build /solana/target/release/ingestor-kafka-hbase .

RUN chmod +x ingestor-kafka-hbase

ENV RUST_LOG=info

ENTRYPOINT ["./ingestor-kafka-hbase"]
