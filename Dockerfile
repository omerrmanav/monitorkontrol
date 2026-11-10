# Yalnızca DERLEME ortamı sağlar (Docker içinden ekranı/I2C'yi yönetmek ve
# GNOME Shell'e eklenti yüklemek mümkün değildir; bunlar ana sistemde çalışır).
FROM rust:1-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends \
        libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM scratch AS export
COPY --from=build /src/target/release/monitorkontrol /monitorkontrol
