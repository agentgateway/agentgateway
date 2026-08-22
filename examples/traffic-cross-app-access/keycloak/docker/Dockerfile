# Package a locally-built Keycloak dist (with ID-JAG support) into a container image shaped
# exactly like an official Keycloak release image: ubi9-micro runtime, Java 21, kc.sh
# entrypoint, /opt/keycloak layout, non-root uid 1000, ports 8080/8443/9000.
#
# Differences from the stock Keycloak container image:
#   - installs java-21 in the build stage so we can run `kc.sh build`
#   - bakes the experimental `identity-assertion-jwt` feature into an optimized build
#   - reads the dist from a LOCAL tarball placed in the build context by build.sh
FROM registry.access.redhat.com/ubi9 AS ubi-micro-build

ARG KEYCLOAK_VERSION=999.0.0-SNAPSHOT
# Local dist tarball in the build context (build.sh copies it here).
ARG KEYCLOAK_DIST=keycloak-${KEYCLOAK_VERSION}.tar.gz

# tar/gzip to unpack; java to run `kc.sh build`
RUN dnf install -y tar gzip java-21-openjdk-headless

# ADD auto-extracts a local tar archive into the destination directory.
ADD $KEYCLOAK_DIST /tmp/keycloak/
# Uniform for local (already extracted by ADD) vs remote (still a tarball) inputs.
RUN (cd /tmp/keycloak && tar -xf /tmp/keycloak/keycloak-*.tar.gz && rm -f /tmp/keycloak/keycloak-*.tar.gz) || true
RUN mv /tmp/keycloak/keycloak-* /opt/keycloak && mkdir -p /opt/keycloak/data
RUN chmod -R g+rwX /opt/keycloak

# Bake the ID-JAG feature into an optimized build. DB stays the default (dev-file); for a
# production image rebuild with e.g. --build-arg and KC_DB=postgres.
ENV KC_FEATURES=identity-assertion-jwt
RUN /opt/keycloak/bin/kc.sh build

# Trim a minimal runtime rootfs (java + langpack) for the micro final image.
ADD ubi-null.sh /tmp/
RUN bash /tmp/ubi-null.sh java-21-openjdk-headless glibc-langpack-en findutils

FROM registry.access.redhat.com/ubi9-micro
ENV LANG=en_US.UTF-8
# Flag for determining app is running in a container
ENV KC_RUN_IN_CONTAINER=true
# Keep the feature on at runtime too (dev mode reads it; matches the optimized build above)
ENV KC_FEATURES=identity-assertion-jwt

COPY --from=ubi-micro-build /tmp/null/rootfs/ /
COPY --from=ubi-micro-build --chown=1000:0 /opt/keycloak /opt/keycloak

RUN echo "keycloak:x:0:root" >> /etc/group && \
    echo "keycloak:x:1000:0:keycloak user:/opt/keycloak:/sbin/nologin" >> /etc/passwd

USER 1000
EXPOSE 8080
EXPOSE 8443
EXPOSE 9000

ENTRYPOINT [ "/opt/keycloak/bin/kc.sh" ]

ARG KEYCLOAK_VERSION
LABEL org.opencontainers.image.title="Keycloak (ID-JAG build)" \
      org.opencontainers.image.description="Keycloak with the identity-assertion-jwt (ID-JAG) feature" \
      version=${KEYCLOAK_VERSION}
