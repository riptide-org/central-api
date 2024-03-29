FROM lukemathwalker/cargo-chef:latest-rust-latest AS chef
WORKDIR /app

FROM chef as planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update
RUN apt-get install -y cmake
RUN wget https://github.com/protocolbuffers/protobuf/releases/download/v3.20.0/protoc-3.20.0-linux-x86_64.zip
RUN unzip protoc-3.20.0-linux-x86_64.zip
RUN cp -r include/* /usr/local/include
RUN cp bin/protoc /usr/bin/

COPY --from=planner /app/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build application
COPY . .
RUN cargo build --release

# We do not need the Rust toolchain to run the binary!
FROM debian:bullseye-slim AS runtime
WORKDIR /app
COPY --from=builder /app/target/release/central_api /app/
COPY --from=builder /app/migrations /app/migrations

RUN apt-get update
RUN apt-get upgrade -y
RUN apt-get install -y sqlite3

ENV HOST 0.0.0.0
ENV PORT 3000

EXPOSE 3000

CMD [ "/app/central_api" ]
