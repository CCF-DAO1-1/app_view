FROM m.daocloud.io/docker.io/rust:slim-bullseye AS buildstage
WORKDIR /build
ENV PROTOC_NO_VENDOR 1
RUN rustup component add rustfmt && \
    apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev  && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*
COPY . /build/
RUN cargo build --release

FROM m.daocloud.io/docker.io/rust:slim-bullseye
RUN useradd -m dao
USER dao
COPY --from=buildstage /build/target/release/dao /usr/bin/
CMD ["dao", "--db-url $DB_URL", "--pds $PDS"]
