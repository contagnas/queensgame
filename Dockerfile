FROM nixos/nix:2.24.11 AS builder

WORKDIR /app
COPY . .

RUN nix --extra-experimental-features "nix-command flakes" \
    --option sandbox false \
    build .#default --print-build-logs

RUN mkdir -p /tmp/nix-store-closure \
    && cp -a $(nix-store -qR result) /tmp/nix-store-closure/

FROM nixos/nix:2.24.11

WORKDIR /app
COPY --from=builder /tmp/nix-store-closure/ /nix/store/
COPY --from=builder /app/result /app/result

ENV RUST_LOG=info

CMD ["/app/result/bin/queensgame"]
