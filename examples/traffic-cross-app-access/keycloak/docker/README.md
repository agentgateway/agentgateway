# Keycloak ID-JAG container image

Packages a locally-built Keycloak (with ID-JAG support) into a container image shaped like a
normal Keycloak release (ubi9-micro, Java 21, `kc.sh` entrypoint, `/opt/keycloak`, non-root
uid 1000, ports 8080/8443/9000), with the experimental **`identity-assertion-jwt`** feature
baked into an optimized build. You configure it exactly the way we do outside Docker — the
`setup.sh` / `setup-leg2.sh` scripts work unchanged against the container.

## Prerequisites

- Docker running.
- A locally-built Keycloak dist tarball with ID-JAG support:
  `~/java/keycloak/quarkus/dist/target/keycloak-999.0.0-SNAPSHOT.tar.gz`
  (build: `cd ~/java/keycloak && ./mvnw -pl quarkus/deployment,quarkus/dist -am -DskipTests clean install`)

## 1. Build the image

```bash
cd ~/rust/agentgateway-idjag/examples/traffic-cross-app-access/keycloak/docker
./build.sh            # -> keycloak-idjag:999.0.0-SNAPSHOT
```
Overrides: `KC_DIST_SRC=/path/to/keycloak-*.tar.gz IMAGE=my/keycloak:tag ./build.sh`.

## 2. Run the container

Dev mode (H2, matches how we run it locally). Run Keycloak on `:8480` and map `8480:8480` so
the **same URL is used inside and outside** the container:

```bash
docker run --name kc-idjag --rm -p 8480:8480 \
  -e KC_BOOTSTRAP_ADMIN_USERNAME=admin -e KC_BOOTSTRAP_ADMIN_PASSWORD=admin \
  keycloak-idjag:999.0.0-SNAPSHOT start-dev --http-port=8480
```

The `identity-assertion-jwt` feature is already enabled (baked via `KC_FEATURES` in the image),
so no `--features` flag is needed. Admin console: <http://localhost:8480> (admin/admin).

> **Why `8480:8480` and `--http-port=8480` (not `8480:8080`)?** ID-JAG is self-referential: the
> `self-idjag` IdP's issuer, the token `iss`, and the IdP's JWKS fetch must all agree on one base
> URL. If the container listened on `:8080` internally but tokens were minted via the host `:8480`,
> the issuer wouldn't match the IdP and leg 2 fails with *"No Identity Provider for provided
> issuer"*. Using one port everywhere avoids the split. (A production deployment pins this with a
> fixed `KC_HOSTNAME` instead.)

> Production-style instead: `... keycloak-idjag:999.0.0-SNAPSHOT start --optimized --http-port=8480 --hostname=http://localhost:8480 --http-enabled=true`
> (the image was `kc.sh build`-optimized with the feature; add a real `KC_DB`/hostname for prod).

## 3. Configure it — the same scripts as always

The scripts live one directory up. Two ways to point `kcadm` at the container:

**A. Container-native (no Keycloak install on the host)** — uses `docker/kcadm.sh`, which runs
`kcadm` inside the container. With the container on `:8480`, the server URL is the same `:8480`:

```bash
cd ..
KCADM="$PWD/docker/kcadm.sh" SERVER=http://localhost:8480 ./configure-keycloak.sh
```

**B. Host kcadm against the mapped port** — if you already have a Keycloak dist unpacked:

```bash
cd ..
KCADM=/tmp/kc-idjag/bin/kcadm.sh ./configure-keycloak.sh   # SERVER defaults to :8480
```

Either creates realm `idjag-demo`, `alice`/`alice`, `agent-client`, `resource-client`, the
`self-idjag` IdP, the federated link, and the `todos.*` scopes — identical to the non-Docker flow.

## 4. Drive the flow

Point agentgateway at `http://localhost:8480/...` exactly as in [`../README.md`](../README.md)
(the gateway config and `round-trip.sh` are unchanged — Keycloak just runs in a container now):

```bash
cd ..
./round-trip.sh          # ID token -> ID-JAG -> Bearer access token
# or through the gateway:
../../../target/release/agentgateway -f ./gateway.yaml    # :3030
```

## Notes

- **Persistence:** dev mode uses an ephemeral H2 db inside the container; `--rm` wipes it on stop.
  Add `-v kc-idjag-data:/opt/keycloak/data` to keep the realm across restarts.
- **Config baked at build time:** features (and DB vendor) are set during `kc.sh build` in the
  image. To change them, edit the `Dockerfile` `KC_FEATURES` / add `KC_DB` and rebuild.
- The image contains **your** Keycloak build with ID-JAG issuer support — it is not an official
  Keycloak release.
