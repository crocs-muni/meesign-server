# Build meesign helper
FROM maven:3-jdk-11 as java-builder
WORKDIR /
RUN git clone https://github.com/dufkan/meesign-helper.git meesign-helper
RUN cd meesign-helper && mvn clean compile assembly:single


# Build and statically link the meesign binary
FROM kristianmika/rust-musl-builder:stable as rust-builder
WORKDIR /home/rust/src/
ADD --chown=rust:rust . .
RUN cargo build --release --target x86_64-unknown-linux-musl


# Use a clean container to run the binary 
# note it must be a JRE image for meesign helper
FROM eclipse-temurin:11-jre-alpine as runner
COPY --from=rust-builder /home/rust/src/target/x86_64-unknown-linux-musl/release/meesign-server /usr/local/bin/meesign-server
COPY --from=java-builder /meesign-helper/target/signPDF-1.0-SNAPSHOT-jar-with-dependencies.jar /meesign/MeeSignHelper.jar

ARG SERVER_PORT=1337
ARG BUILD_DATE
ARG REVISION
ARG BUILD_VERSION

LABEL org.opencontainers.image.created=${BUILD_DATE} \
      org.opencontainers.image.source="https://github.com/crocs-muni/meesign-server" \
      org.opencontainers.image.version=${BUILD_VERSION} \
      org.opencontainers.image.revision=${REVISION} \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.title="meesign-server" \
      org.opencontainers.image.description="Meesign server for threshold ECDSA signatures." \
      org.opencontainers.image.vendor="CRoCS, FI MUNI" \
      org.label-schema.docker.cmd="docker run --detach --publish 1337:1337 --volume `pwd`/keys/:/meesign/keys/ crocsmuni/meesign:latest"

EXPOSE ${SERVER_PORT}
# running the binary from a specific directory as meesign helper requires 
# a certificate and a private key in the currect directory 
WORKDIR /meesign
ENTRYPOINT ["meesign-server"]
CMD ["--addr", "0.0.0.0"]
